//! Controller: connects UI actions (Slint `Actions` global) to the API and the
//! local working folder, and pushes results back into `AppState`.
//!
//! Threading model: network and file work runs on background threads
//! (`Ctx::run`); results are applied on the UI thread through
//! `slint::invoke_from_event_loop`. Slint models are only touched on the UI thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Result, anyhow, bail};
use slint::{ComponentHandle, Model, ModelRc, SharedString, StandardListViewItem, VecModel};

use crate::api::{self, Api, Baseline, BaselineDetail, Item, Project, User, UserPatch, Version, VersionCtx};
use crate::config::{self, Config};
use crate::dialogs::{self, Dialog};
use crate::workspace::{self, LocalInfo, LocalStatus, Workspace};
use crate::{ActivityRow, Actions, AppState, AppWindow, BaselineRow, ItemDetail, Labels, ProjectRow, UserRow, VersionRow, fmt};

type Confirm = Box<dyn FnOnce(&Ctx) + Send>;

#[derive(Default)]
pub struct State {
    pub config: Config,
    api: Option<Api>,
    me: Option<User>,
    workspace: Option<Workspace>,
    projects: Vec<Project>,
    project_id: Option<i64>,
    items: Vec<Item>,
    locals: Vec<(LocalStatus, PathBuf)>,
    item: Option<Item>,
    local: Option<LocalInfo>,
    versions: Vec<Version>,
    reviews: Vec<VersionCtx>,
    baselines: Vec<Baseline>,
    baseline: Option<BaselineDetail>,
    users: Vec<User>,
    pending_confirm: Option<Confirm>,
}

#[derive(Clone)]
pub struct Ctx {
    ui: slint::Weak<AppWindow>,
    st: Arc<Mutex<State>>,
    busy: Arc<AtomicUsize>,
}

// ------------------------------------------------------------------ helpers

fn s(v: impl AsRef<str>) -> SharedString {
    SharedString::from(v.as_ref())
}

fn list(items: Vec<String>) -> ModelRc<StandardListViewItem> {
    ModelRc::new(VecModel::from(items.into_iter().map(|t| StandardListViewItem::from(s(t))).collect::<Vec<_>>()))
}

fn rows(rows: Vec<Vec<String>>) -> ModelRc<ModelRc<StandardListViewItem>> {
    ModelRc::new(VecModel::from(rows.into_iter().map(list).collect::<Vec<_>>()))
}

fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn bump_order(kind: &str) -> &'static [&'static str] {
    if kind == "cad" { &["revision", "iteration"] } else { &["patch", "minor", "major"] }
}

/// File name used by "Salvar como" / baseline export: manga_revB.dwg, ecu_v1.2.0.hex
fn export_name(item_file_name: &str, kind: &str, v: &Version) -> String {
    let p = Path::new(item_file_name);
    let stem = p.file_stem().and_then(|x| x.to_str()).unwrap_or("arquivo");
    let ext = Path::new(&v.filename).extension().and_then(|x| x.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    let tag = if kind == "cad" { format!("rev{}", v.label) } else { format!("v{}", v.label) };
    workspace::sanitize(&format!("{stem}_{tag}{ext}"))
}

impl Ctx {
    pub fn new(ui: &AppWindow) -> Self {
        let config = Config::load();
        let ctx = Ctx { ui: ui.as_weak(), st: Arc::new(Mutex::new(State { config, ..Default::default() })), busy: Default::default() };
        ctx.wire(ui);
        ctx
    }

    /// Lock shared state. Never call twice in one expression (std Mutex is not
    /// re-entrant: that deadlocks the UI thread). Bind values to locals first.
    fn st(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn api(&self) -> Result<Api> {
        self.st().api.clone().ok_or_else(|| anyhow!("Não conectado."))
    }

    /// Run `work` on a background thread, then `done` on the UI thread.
    fn run<T: Send + 'static>(
        &self,
        status: &str,
        work: impl FnOnce(&Ctx) -> Result<T> + Send + 'static,
        done: impl FnOnce(&Ctx, &AppWindow, T) + Send + 'static,
    ) {
        let quiet = status.is_empty();
        if let Some(ui) = self.ui.upgrade() {
            if !quiet {
                self.busy.fetch_add(1, Ordering::SeqCst);
                let g = ui.global::<AppState>();
                g.set_busy(true);
                g.set_status_text(s(status));
                g.set_error_text(s(""));
            }
        }
        let ctx = self.clone();
        std::thread::spawn(move || {
            let result = work(&ctx);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = ctx.ui.upgrade() else { return };
                if !quiet && ctx.busy.fetch_sub(1, Ordering::SeqCst) == 1 {
                    ui.global::<AppState>().set_busy(false);
                }
                match result {
                    Ok(v) => done(&ctx, &ui, v),
                    Err(e) => ctx.show_error(&ui, &e, quiet),
                }
            });
        });
    }

    fn show_error(&self, ui: &AppWindow, e: &anyhow::Error, quiet: bool) {
        let g = ui.global::<AppState>();
        if api::is_unauthorized(e) && g.get_page() == "main" {
            self.to_login(ui, "Sua sessão expirou. Entre novamente.");
            return;
        }
        if quiet {
            return; // background auto-refresh: don't nag
        }
        g.set_status_text(s(""));
        g.set_error_text(s(format!("{e:#}")));
    }

    fn info(ui: &AppWindow, msg: impl AsRef<str>) {
        let g = ui.global::<AppState>();
        g.set_error_text(s(""));
        g.set_status_text(s(msg));
    }

    fn confirm(&self, ui: &AppWindow, title: &str, text: &str, ok: &str, action: impl FnOnce(&Ctx) + Send + 'static) {
        self.st().pending_confirm = Some(Box::new(action));
        let g = ui.global::<AppState>();
        g.set_confirm_title(s(title));
        g.set_confirm_text(s(text));
        g.set_confirm_ok_text(s(ok));
        g.set_dialog(s("confirm"));
    }

    // ------------------------------------------------------------------ wiring

    fn wire(&self, ui: &AppWindow) {
        let a = ui.global::<Actions>();
        macro_rules! on {
            ($name:ident, |$c:ident $(, $arg:ident)*| $body:expr) => {{
                let $c = self.clone();
                a.$name(move |$($arg),*| { $body; });
            }};
        }
        on!(on_login, |c, url, user, pass| c.login(url.to_string(), user.to_string(), pass.to_string()));
        on!(on_logout, |c| c.logout());
        on!(on_refresh, |c| c.refresh(false));
        on!(on_show_view, |c, view| c.show_view(&view));
        on!(on_select_project, |c, i| c.select_project(i));
        on!(on_select_item, |c, row| c.select_item(row));
        on!(on_select_version, |c, row| c.with_ui(|ui| c.sync_version_status(ui, row)));
        on!(on_create_project, |c, n, k, sub, d, l| c.create_project(n.into(), k.into(), sub.into(), d.into(), l));
        on!(on_update_project, |c, d, l, ar| c.update_project(d.into(), l, ar));
        on!(on_pick_file, |c| c.pick_file());
        on!(on_create_item, |c, n, pn, d, cm, b| c.create_item(n.into(), pn.into(), d.into(), cm.into(), b.into()));
        on!(on_checkout, |c, note| c.checkout(note.into()));
        on!(on_checkin, |c, cm, b, keep| c.checkin(cm.into(), b.into(), keep));
        on!(on_undo_checkout, |c| c.undo_checkout());
        on!(on_get_latest, |c| c.get_latest());
        on!(on_sync_project, |c| c.sync_project());
        on!(on_open_file, |c| c.open_file());
        on!(on_open_folder, |c| c.open_folder());
        on!(on_save_version_as, |c| c.save_version_as());
        on!(on_set_version_status, |c, st| c.set_version_status(st.into()));
        on!(on_review_set_status, |c, st| c.review_set_status(st.into()));
        on!(on_review_open_item, |c| c.review_open_item());
        on!(on_create_baseline, |c, n, d, r| c.create_baseline(n.into(), d.into(), r));
        on!(on_select_baseline, |c, i| c.select_baseline(i));
        on!(on_download_baseline, |c| c.download_baseline());
        on!(on_create_user, |c, u, n, p, r, sub| c.create_user(u.into(), n.into(), p.into(), r.into(), sub.into()));
        on!(on_update_user, |c, r, sub, act| c.update_user(r.into(), sub.into(), act));
        on!(on_reset_password, |c, p| c.reset_password(p.into()));
        on!(on_choose_workspace, |c| c.choose_workspace());
        on!(on_set_workspace, |c, p| c.set_workspace(PathBuf::from(p.as_str())));
        on!(on_set_open_after_checkout, |c, v| c.set_open_after_checkout(v));
        on!(on_change_password, |c, cur, new| c.change_password(cur.into(), new.into()));
        on!(on_confirm_accepted, |c| c.confirm_accepted());
        on!(on_confirm_cancelled, |c| c.confirm_cancelled());

        // Prefill login form and try the saved session.
        let cfg = self.st().config.clone();
        let g = ui.global::<AppState>();
        g.set_client_version(s(env!("CARGO_PKG_VERSION")));
        g.set_server_url(s(&cfg.server_url));
        g.set_username(s(&cfg.username));
        g.set_open_after_checkout(cfg.open_after_checkout);
        g.set_workspace_path(s(cfg.workspace_dir().to_string_lossy()));
        if cfg.token.is_some() && !cfg.server_url.is_empty() {
            self.resume_session();
        }
    }

    fn with_ui(&self, f: impl FnOnce(&AppWindow)) {
        if let Some(ui) = self.ui.upgrade() {
            f(&ui);
        }
    }

    /// Silent periodic refresh (other people's check-outs, new versions).
    pub fn auto_refresh(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let g = ui.global::<AppState>();
        if g.get_page() == "main" && !g.get_busy() && g.get_dialog().is_empty() {
            self.refresh(true);
        }
    }

    // ------------------------------------------------------------------ session

    fn resume_session(&self) {
        self.run(
            "Reconectando…",
            |c| {
                let cfg = c.st().config.clone();
                let api = Api::new(&cfg.server_url, cfg.token.clone())?;
                let meta = api.meta()?;
                let me = api.me()?;
                Ok((api, meta, me))
            },
            |c, ui, (api, meta, me)| c.enter(ui, api, meta, me),
        );
    }

    fn login(&self, url: String, username: String, password: String) {
        if url.trim().is_empty() || username.trim().is_empty() {
            self.with_ui(|ui| ui.global::<AppState>().set_error_text(s("Informe o servidor e o usuário.")));
            return;
        }
        self.run(
            "Conectando…",
            move |_| {
                let mut api = Api::new(&url, None)?;
                let meta = api.meta()?;
                if !meta.has_users {
                    bail!("Este servidor ainda não tem usuários. Abra {} no navegador para criar o administrador.", api.base_url());
                }
                let out = api.login(username.trim(), &password, &config::device_name())?;
                Ok((api, meta, out.user))
            },
            |c, ui, (api, meta, me)| {
                {
                    let mut st = c.st();
                    st.config.server_url = api.base_url().to_owned();
                    st.config.username = me.username.clone();
                    st.config.token = api.token().map(str::to_owned);
                    let _ = st.config.save();
                }
                c.enter(ui, api, meta, me);
            },
        );
    }

    fn enter(&self, ui: &AppWindow, api: Api, meta: api::Meta, me: User) {
        let ws_dir = self.st().config.workspace_dir();
        match Workspace::open(&ws_dir, api.base_url()) {
            Ok(ws) => self.st().workspace = Some(ws),
            Err(e) => self.show_error(ui, &e, false),
        }
        let g = ui.global::<AppState>();
        g.set_server_url(s(api.base_url()));
        g.set_team_name(s(&meta.team_name));
        g.set_server_version(s(&meta.server_version));
        g.set_subsystems(model(meta.subsystems.iter().map(s).collect()));
        g.set_me_name(s(&me.name));
        g.set_me_role(s(&me.role));
        g.set_is_admin(me.is_admin());
        g.set_can_write(me.can_write());
        g.set_workspace_path(s(ws_dir.to_string_lossy()));
        g.set_error_text(s(""));
        g.set_page(s("main"));
        g.set_view(s("project"));
        {
            let mut st = self.st();
            st.api = Some(api);
            st.me = Some(me);
        }
        self.refresh(false);
    }

    fn to_login(&self, ui: &AppWindow, msg: &str) {
        {
            let mut st = self.st();
            st.api = None;
            st.me = None;
            st.config.token = None;
            let _ = st.config.save();
        }
        let g = ui.global::<AppState>();
        g.set_page(s("login"));
        g.set_dialog(s(""));
        g.set_error_text(s(msg));
    }

    fn logout(&self) {
        let api = self.st().api.clone();
        std::thread::spawn(move || {
            if let Some(api) = api {
                let _ = api.logout();
            }
        });
        self.with_ui(|ui| self.to_login(ui, ""));
    }

    // ------------------------------------------------------------------ loading

    /// Reload projects + review count, then the current project.
    fn refresh(&self, quiet: bool) {
        self.run(
            if quiet { "" } else { "Atualizando…" },
            |c| {
                let api = c.api()?;
                let include_archived = c.st().me.as_ref().is_some_and(User::is_admin);
                Ok((api.projects(include_archived)?, api.reviews()?))
            },
            move |c, ui, (projects, reviews)| {
                c.apply_projects(ui, projects);
                c.apply_reviews(ui, reviews);
                let g = ui.global::<AppState>();
                match g.get_view().as_str() {
                    "activity" => c.load_activity(),
                    "admin" => c.load_users(),
                    _ => {}
                }
                let (pid, iid) = { let st = c.st(); (st.project_id, st.item.as_ref().map(|i| i.id)) };
                if let Some(pid) = pid {
                    c.load_project(pid, iid, quiet);
                }
                if !quiet {
                    Ctx::info(ui, "Atualizado.");
                }
            },
        );
    }

    fn apply_projects(&self, ui: &AppWindow, projects: Vec<Project>) {
        let g = ui.global::<AppState>();
        let labels = ui.global::<Labels>();
        let mut st = self.st();
        let names = projects
            .iter()
            .map(|p| {
                let arch = if p.archived { " (arquivado)" } else { "" };
                format!("{}  ·  {}{}", p.name, labels.invoke_kind(s(&p.kind)), arch)
            })
            .collect();
        g.set_project_list(list(names));
        g.set_projects(model(projects.iter().map(project_row).collect()));
        if st.project_id.is_none_or(|id| !projects.iter().any(|p| p.id == id)) {
            st.project_id = projects.first().map(|p| p.id);
            st.item = None;
        }
        let idx = st.project_id.and_then(|id| projects.iter().position(|p| p.id == id));
        g.set_current_project_index(idx.map_or(-1, |i| i as i32));
        g.set_has_project(idx.is_some());
        if let Some(i) = idx {
            g.set_project(project_row(&projects[i]));
        }
        st.projects = projects;
    }

    fn select_project(&self, index: i32) {
        let pid = { self.st().projects.get(index as usize).map(|p| p.id) };
        let Some(pid) = pid else { return };
        {
            let mut st = self.st();
            if st.project_id != Some(pid) {
                st.item = None;
            }
            st.project_id = Some(pid);
        }
        self.with_ui(|ui| {
            let g = ui.global::<AppState>();
            g.set_view(s("project"));
            if let Some(p) = self.st().projects.get(index as usize) {
                g.set_project(project_row(p));
                g.set_has_project(true);
            }
            g.set_current_project_index(index);
            g.set_has_item(false);
            g.set_current_item_row(-1);
            g.set_current_baseline_index(-1);
            g.set_baseline_entry_rows(rows(vec![]));
        });
        self.load_project(pid, None, false);
    }

    fn load_project(&self, pid: i64, keep_item: Option<i64>, quiet: bool) {
        self.run(
            if quiet { "" } else { "Carregando projeto…" },
            move |c| {
                let api = c.api()?;
                let items = api.items(pid)?;
                let baselines = api.baselines(pid)?;
                let locals = c.inspect_all(&items);
                Ok((items, baselines, locals))
            },
            move |c, ui, (items, baselines, locals)| {
                if c.st().project_id != Some(pid) {
                    return; // user moved on
                }
                c.apply_items(ui, items, locals);
                c.apply_baselines(ui, baselines);
                let row = keep_item.and_then(|id| c.st().items.iter().position(|i| i.id == id));
                let g = ui.global::<AppState>();
                match row {
                    Some(r) => {
                        g.set_current_item_row(r as i32);
                        let id = c.st().items[r].id;
                        c.load_item(id, quiet);
                    }
                    None => {
                        g.set_current_item_row(-1);
                        g.set_has_item(false);
                    }
                }
                if !quiet {
                    Ctx::info(ui, "");
                }
            },
        );
    }

    fn inspect_all(&self, items: &[Item]) -> Vec<(LocalStatus, PathBuf)> {
        let mut st = self.st();
        match st.workspace.as_mut() {
            Some(ws) => items.iter().map(|i| { let l = ws.inspect(i); (l.status, l.path) }).collect(),
            None => items.iter().map(|_| (LocalStatus::Missing, PathBuf::new())).collect(),
        }
    }

    fn lock_text(&self, item: &Item) -> String {
        let me = self.st().me.as_ref().map(|u| u.id);
        match &item.locked_by {
            None => "Livre".into(),
            Some(u) if Some(u.id) == me => "Com você".into(),
            Some(u) => format!("Com {}", u.name),
        }
    }

    fn item_row(&self, ui: &AppWindow, item: &Item, local: LocalStatus) -> Vec<String> {
        let labels = ui.global::<Labels>();
        let (vl, status) = item.latest_version.as_ref().map_or((String::new(), String::new()), |v| {
            (fmt::version_label(&item.project_kind, &v.label), labels.invoke_status(s(&v.status), s(&item.project_kind)).to_string())
        });
        vec![
            item.part_number.clone().unwrap_or_default(),
            item.name.clone(),
            vl,
            status,
            self.lock_text(item),
            labels.invoke_local(s(local.key())).to_string(),
        ]
    }

    fn apply_items(&self, ui: &AppWindow, items: Vec<Item>, locals: Vec<(LocalStatus, PathBuf)>) {
        let table = items.iter().zip(&locals).map(|(i, l)| self.item_row(ui, i, l.0)).collect();
        let g = ui.global::<AppState>();
        g.set_item_rows(rows(table));
        let mut st = self.st();
        let me = st.me.as_ref().map(|u| u.id);
        g.set_my_lock_count(items.iter().filter(|i| i.locked_by.as_ref().map(|u| u.id) == me && me.is_some()).count() as i32);
        st.items = items;
        st.locals = locals;
    }

    fn select_item(&self, row: i32) {
        let id = self.st().items.get(row as usize).map(|i| i.id);
        if let Some(id) = id {
            self.load_item(id, false);
        }
    }

    fn load_item(&self, item_id: i64, quiet: bool) {
        self.run(
            if quiet { "" } else { "Carregando item…" },
            move |c| {
                let api = c.api()?;
                let item = api.item(item_id)?;
                let versions = api.versions(item_id)?;
                let activity = api.activity(None, Some(item_id), 20)?;
                let local = {
                    let mut st = c.st();
                    st.workspace.as_mut().map(|ws| ws.inspect(&item))
                };
                Ok((item, versions, activity, local))
            },
            move |c, ui, (item, versions, activity, local)| {
                c.apply_item(ui, item, versions, local);
                ui.global::<AppState>().set_item_activity(model(activity.iter().map(|a| activity_row(ui, a)).collect()));
                if !quiet {
                    Ctx::info(ui, "");
                }
            },
        );
    }

    fn apply_item(&self, ui: &AppWindow, item: Item, versions: Vec<Version>, local: Option<LocalInfo>) {
        let g = ui.global::<AppState>();
        let labels = ui.global::<Labels>();
        let me = self.st().me.as_ref().map(|u| u.id);
        let kind = item.project_kind.clone();
        let latest = item.latest_version.as_ref();
        let order = bump_order(&kind);
        let bump_labels: Vec<SharedString> = order
            .iter()
            .map(|k| {
                let next = item.next_labels.get(*k).cloned().unwrap_or_default();
                s(format!("{}  —  {}", fmt::version_label(&kind, &next), labels.invoke_bump(s(k))))
            })
            .collect();
        let local_status = local.as_ref().map_or(LocalStatus::Missing, |l| l.status);
        let detail = ItemDetail {
            id: item.id as i32,
            name: s(&item.name),
            part_number: s(item.part_number.as_deref().unwrap_or("")),
            description: s(&item.description),
            kind: s(&kind),
            file_name: s(&item.file_name),
            version_label: s(latest.map(|v| fmt::version_label(&kind, &v.label)).unwrap_or_default()),
            status_label: latest.map(|v| labels.invoke_status(s(&v.status), s(&kind))).unwrap_or_default(),
            version_count: item.version_count as i32,
            locked: item.locked_by.is_some(),
            locked_by_me: item.locked_by.as_ref().is_some_and(|u| Some(u.id) == me),
            locked_by: s(item.locked_by.as_ref().map(|u| u.name.as_str()).unwrap_or("")),
            locked_at: s(item.locked_at.as_deref().map(fmt::date).unwrap_or_default()),
            lock_note: s(item.lock_note.as_deref().unwrap_or("")),
            require_lock: item.require_lock,
            local_status: s(local_status.key()),
            local_path: s(local.as_ref().map(|l| l.path.to_string_lossy().to_string()).unwrap_or_default()),
            base_label: s(local.as_ref().and_then(|l| l.base_label.as_ref()).map(|b| fmt::version_label(&kind, b)).unwrap_or_default()),
            bump_keys: model(order.iter().map(s).collect()),
            bump_labels: model(bump_labels),
            default_bump: if kind == "cad" { 0 } else { 1 },
        };
        let vrows = versions
            .iter()
            .map(|v| {
                vec![
                    fmt::version_label(&kind, &v.label),
                    labels.invoke_status(s(&v.status), s(&kind)).to_string(),
                    v.comment.clone(),
                    v.author.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                    fmt::date(&v.created_at),
                    fmt::size(v.size),
                    v.baselines.iter().map(|b| b.name.as_str()).collect::<Vec<_>>().join(", "),
                ]
            })
            .collect();
        let prev_row = g.get_current_version_row();
        g.set_versions(model(versions.iter().map(|v| VersionRow { id: v.id as i32, label: s(&v.label), status: s(&v.status), comment: s(&v.comment) }).collect()));
        g.set_version_rows(rows(vrows));
        let same_item = self.st().item.as_ref().is_some_and(|i| i.id == item.id);
        let row = if same_item && prev_row >= 0 && (prev_row as usize) < versions.len() { prev_row } else if versions.is_empty() { -1 } else { 0 };
        g.set_current_version_row(row);
        g.set_version_status_index(versions.get(row.max(0) as usize).map_or(0, |v| status_index(&v.status)));
        g.set_item(detail);
        g.set_has_item(true);

        // Keep the items table row in sync.
        let mut st = self.st();
        if let Some(pos) = st.items.iter().position(|i| i.id == item.id) {
            g.set_current_item_row(pos as i32);
            st.items[pos] = item.clone();
            if let Some(l) = &local {
                st.locals[pos] = (l.status, l.path.clone());
            }
            drop(st);
            let row = self.item_row(ui, &item, local_status);
            let table = g.get_item_rows();
            if let Some(vm) = table.as_any().downcast_ref::<VecModel<ModelRc<StandardListViewItem>>>() {
                vm.set_row_data(pos, list(row));
            }
            st = self.st();
        }
        st.item = Some(item);
        st.local = local;
        st.versions = versions;
    }

    fn sync_version_status(&self, ui: &AppWindow, row: i32) {
        let g = ui.global::<AppState>();
        g.set_current_version_row(row);
        if let Some(v) = self.st().versions.get(row as usize) {
            g.set_version_status_index(status_index(&v.status));
        }
    }

    fn reload_current_item(&self) {
        let id = self.st().item.as_ref().map(|i| i.id);
        if let Some(id) = id {
            self.load_item(id, false);
        }
    }

    // ------------------------------------------------------------------ views

    fn show_view(&self, view: &str) {
        self.with_ui(|ui| ui.global::<AppState>().set_view(s(view)));
        match view {
            "reviews" => self.load_reviews(),
            "activity" => self.load_activity(),
            "admin" => self.load_users(),
            _ => {}
        }
    }

    fn load_reviews(&self) {
        self.run("Carregando revisões…", |c| c.api()?.reviews(), |c, ui, r| {
            c.apply_reviews(ui, r);
            Ctx::info(ui, "");
        });
    }

    fn apply_reviews(&self, ui: &AppWindow, reviews: Vec<VersionCtx>) {
        let g = ui.global::<AppState>();
        let table = reviews
            .iter()
            .map(|r| {
                vec![
                    r.project_name.clone(),
                    r.item_name.clone(),
                    fmt::version_label(&r.project_kind, &r.version.label),
                    r.version.comment.clone(),
                    r.version.author.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                    fmt::date(&r.version.created_at),
                ]
            })
            .collect();
        g.set_review_rows(rows(table));
        g.set_review_count(reviews.len() as i32);
        if g.get_current_review_row() >= reviews.len() as i32 {
            g.set_current_review_row(-1);
        }
        self.st().reviews = reviews;
    }

    fn review_set_status(&self, status: String) {
        let Some(ui) = self.ui.upgrade() else { return };
        let row = ui.global::<AppState>().get_current_review_row();
        let Some(vid) = self.st().reviews.get(row as usize).map(|r| r.version.id) else { return };
        self.run("Alterando status…", move |c| { c.api()?.set_status(vid, &status)?; c.api()?.reviews() }, |c, ui, r| {
            c.apply_reviews(ui, r);
            Ctx::info(ui, "Status alterado.");
        });
    }

    fn review_open_item(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let row = ui.global::<AppState>().get_current_review_row();
        let Some((pid, iid)) = self.st().reviews.get(row as usize).map(|r| (r.project_id, r.version.item_id)) else { return };
        let idx = self.st().projects.iter().position(|p| p.id == pid);
        if let Some(idx) = idx {
            self.select_project(idx as i32);
            self.load_project(pid, Some(iid), false);
        }
    }

    fn load_activity(&self) {
        self.run("Carregando atividade…", |c| c.api()?.activity(None, None, 200), |_, ui, acts| {
            ui.global::<AppState>().set_activity(model(acts.iter().map(|a| activity_row(ui, a)).collect()));
            Ctx::info(ui, "");
        });
    }

    // ------------------------------------------------------------------ projects & items

    fn create_project(&self, name: String, kind: String, subsystem: String, description: String, lock: bool) {
        self.run(
            "Criando projeto…",
            move |c| c.api()?.create_project(&name, &kind, &subsystem, &description, lock),
            |c, ui, p| {
                ui.global::<AppState>().set_dialog(s(""));
                c.st().project_id = Some(p.id);
                c.st().item = None;
                c.refresh(false);
                Ctx::info(ui, format!("Projeto “{}” criado.", p.name));
            },
        );
    }

    fn update_project(&self, description: String, lock: bool, archived: bool) {
        let Some(pid) = self.st().project_id else { return };
        self.run(
            "Salvando projeto…",
            move |c| c.api()?.patch_project(pid, serde_json::json!({"description": description, "require_lock": lock, "archived": archived})),
            |c, ui, _| {
                ui.global::<AppState>().set_dialog(s(""));
                c.refresh(false);
            },
        );
    }

    fn pick_file(&self) {
        let ui = self.ui.clone();
        dialogs::ask(Dialog::PickFile { title: "Escolher arquivo" }, move |f| {
            if let Some(ui) = ui.upgrade() {
                ui.global::<AppState>().set_picked_file(s(f.to_string_lossy()));
            }
        });
    }

    fn create_item(&self, name: String, part: String, desc: String, comment: String, bump: String) {
        let Some(ui) = self.ui.upgrade() else { return };
        let src = PathBuf::from(ui.global::<AppState>().get_picked_file().as_str());
        let Some(pid) = self.st().project_id else { return };
        self.run(
            "Enviando primeira versão…",
            move |c| {
                let api = c.api()?;
                let item = api.create_item(pid, &name, &part, &desc, &comment, &bump, &src)?;
                // Put the file in the working folder as the base of version 1.
                if let Some(v) = &item.latest_version {
                    let mut st = c.st();
                    if let Some(ws) = st.workspace.as_mut() {
                        let dest = ws.item_path(&item);
                        if dest != src {
                            if let Some(dir) = dest.parent() {
                                std::fs::create_dir_all(dir)?;
                            }
                            if !dest.exists() {
                                std::fs::copy(&src, &dest)?;
                            }
                        }
                        if workspace::hash_file(&dest).ok().as_deref() == Some(v.sha256.as_str()) {
                            ws.record(&item, v, &v.sha256)?;
                            workspace::set_writable(&dest, !item.require_lock)?;
                        }
                    }
                }
                Ok(item)
            },
            |c, ui, item| {
                ui.global::<AppState>().set_dialog(s(""));
                c.load_project(item.project_id, Some(item.id), false);
                Ctx::info(ui, format!("Item “{}” criado.", item.name));
            },
        );
    }

    // ------------------------------------------------------------------ working copy

    fn current(&self) -> Option<(Item, LocalInfo)> {
        let mut st = self.st();
        let item = st.item.clone()?;
        let local = st.workspace.as_mut()?.inspect(&item);
        Some((item, local))
    }

    /// Download the latest version into the working folder; returns a backup path if
    /// a locally modified file had to be moved aside.
    fn fetch_latest(&self, item: &Item, local: &LocalInfo, writable: bool) -> Result<Option<PathBuf>> {
        let Some(v) = &item.latest_version else { return Ok(None) };
        let api = self.api()?;
        let mut backup = None;
        if local.status.is_modified() {
            let st = self.st();
            backup = Some(st.workspace.as_ref().ok_or_else(|| anyhow!("Sem pasta de trabalho."))?.backup(&local.path)?);
        }
        if local.status != LocalStatus::UpToDate || backup.is_some() {
            let sha = api.download(v.id, &local.path)?;
            let mut st = self.st();
            if let Some(ws) = st.workspace.as_mut() {
                ws.record(item, v, &sha)?;
            }
        }
        workspace::set_writable(&local.path, writable)?;
        Ok(backup)
    }

    fn checkout(&self, note: String) {
        let Some((item, _)) = self.current() else { return };
        let open = self.st().config.open_after_checkout;
        self.run(
            "Fazendo check-out…",
            move |c| {
                let locked = c.api()?.lock(item.id, &note)?;
                let local = { let mut st = c.st(); st.workspace.as_mut().map(|ws| ws.inspect(&locked)) }
                    .ok_or_else(|| anyhow!("Sem pasta de trabalho."))?;
                // Edited copy already based on the latest version: keep the user's work.
                let backup = if local.status == LocalStatus::Modified {
                    workspace::set_writable(&local.path, true)?;
                    None
                } else {
                    c.fetch_latest(&locked, &local, true)?
                };
                if open {
                    let _ = open::that_detached(&local.path);
                }
                Ok((locked.id, backup))
            },
            |c, ui, (id, backup)| {
                ui.global::<AppState>().set_dialog(s(""));
                c.load_item(id, true);
                Ctx::info(ui, match backup {
                    Some(b) => format!("Check-out feito. Sua cópia antiga foi guardada em {}", b.display()),
                    None => "Check-out feito. O arquivo está liberado para edição.".into(),
                });
            },
        );
    }

    fn checkin(&self, comment: String, bump: String, keep_lock: bool) {
        let Some((item, local)) = self.current() else { return };
        self.run(
            "Publicando nova versão…",
            move |c| {
                let kind = &item.project_kind;
                match local.status {
                    LocalStatus::Missing => bail!("O arquivo não está na pasta de trabalho: {}", local.path.display()),
                    LocalStatus::UpToDate | LocalStatus::Outdated => bail!("Nenhuma alteração local para publicar."),
                    LocalStatus::Conflict => bail!(
                        "Sua cópia foi feita a partir da {}, mas o servidor já tem a {}. Como não há mesclagem, \
                         salve suas mudanças com outro nome, baixe a última versão e reaplique-as.",
                        local.base_label.as_deref().map(|b| fmt::version_label(kind, b)).unwrap_or_default(),
                        item.latest_version.as_ref().map(|v| fmt::version_label(kind, &v.label)).unwrap_or_default()
                    ),
                    LocalStatus::Modified | LocalStatus::Untracked => {}
                }
                let api = c.api()?;
                let v = api.publish(item.id, &local.path, &comment, &bump, keep_lock)?;
                let fresh = api.item(item.id)?;
                let sha = workspace::hash_file(&local.path)?;
                if sha != v.sha256 {
                    bail!("O arquivo mudou durante o envio. Feche-o no programa de edição e publique de novo.");
                }
                {
                    let mut st = c.st();
                    if let Some(ws) = st.workspace.as_mut() {
                        ws.record(&fresh, &v, &sha)?;
                    }
                }
                workspace::set_writable(&local.path, keep_lock || !fresh.require_lock)?;
                Ok((fresh, v))
            },
            |c, ui, (item, v)| {
                ui.global::<AppState>().set_dialog(s(""));
                c.load_item(item.id, true);
                c.refresh_reviews_quiet();
                Ctx::info(ui, format!("Versão {} publicada.", fmt::version_label(&item.project_kind, &v.label)));
            },
        );
    }

    fn undo_checkout(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some((item, local)) = self.current() else { return };
        let me = self.st().me.as_ref().map(|u| u.id);
        let mine = item.locked_by.as_ref().is_some_and(|u| Some(u.id) == me);
        let do_it = move |c: &Ctx| {
            c.run(
                "Liberando check-out…",
                move |c| {
                    let unlocked = c.api()?.unlock(item.id)?;
                    let backup = if mine {
                        let local = { let mut st = c.st(); st.workspace.as_mut().map(|ws| ws.inspect(&unlocked)) };
                        match local {
                            Some(l) if l.status != LocalStatus::Missing => c.fetch_latest(&unlocked, &l, !unlocked.require_lock)?,
                            _ => None,
                        }
                    } else {
                        None
                    };
                    Ok((unlocked.id, backup))
                },
                |c, ui, (id, backup)| {
                    c.load_item(id, true);
                    Ctx::info(ui, match backup {
                        Some(b) => format!("Check-out desfeito. Suas alterações foram guardadas em {}", b.display()),
                        None => "Check-out liberado.".into(),
                    });
                },
            )
        };
        if !mine {
            let who = item.locked_by.as_ref().map(|u| u.name.clone()).unwrap_or_default();
            self.confirm(&ui, "Forçar liberação",
                &format!("Liberar o check-out de {who}? Alterações que essa pessoa ainda não publicou podem se perder."),
                "Forçar liberação", do_it);
        } else if local.status.is_modified() {
            self.confirm(&ui, "Desfazer check-out",
                "Você tem alterações não publicadas. Elas serão guardadas em .paddock/backup na pasta de trabalho e o arquivo voltará para a última versão do servidor.",
                "Desfazer check-out", do_it);
        } else {
            do_it(self);
        }
    }

    fn get_latest(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some((item, local)) = self.current() else { return };
        let me = self.st().me.as_ref().map(|u| u.id);
        let writable = !item.require_lock || item.locked_by.as_ref().is_some_and(|u| Some(u.id) == me);
        let modified = local.status.is_modified();
        let do_it = move |c: &Ctx| {
            c.run(
                "Baixando…",
                move |c| c.fetch_latest(&item, &local, writable).map(|b| (item.id, b)),
                |c, ui, (id, backup)| {
                    c.load_item(id, true);
                    Ctx::info(ui, match backup {
                        Some(b) => format!("Última versão baixada. Sua cópia anterior está em {}", b.display()),
                        None => "Última versão baixada.".into(),
                    });
                },
            )
        };
        if modified {
            self.confirm(&ui, "Substituir cópia local",
                "Seu arquivo tem alterações que não estão no servidor. Ele será guardado em .paddock/backup e substituído pela última versão.",
                "Substituir", do_it);
        } else {
            do_it(self);
        }
    }

    fn sync_project(&self) {
        let Some(pid) = self.st().project_id else { return };
        self.run(
            "Sincronizando pasta de trabalho…",
            move |c| {
                let api = c.api()?;
                let items = api.items(pid)?;
                let me = c.st().me.as_ref().map(|u| u.id);
                let (mut done, mut skipped) = (0, 0);
                for item in &items {
                    let local = { let mut st = c.st(); st.workspace.as_mut().map(|ws| ws.inspect(item)) };
                    let Some(local) = local else { continue };
                    let writable = !item.require_lock || item.locked_by.as_ref().is_some_and(|u| Some(u.id) == me);
                    match local.status {
                        LocalStatus::Missing | LocalStatus::Outdated => {
                            c.fetch_latest(item, &local, writable)?;
                            done += 1;
                        }
                        LocalStatus::UpToDate => workspace::set_writable(&local.path, writable)?,
                        _ => skipped += 1,
                    }
                }
                Ok((done, skipped))
            },
            move |c, ui, (done, skipped)| {
                let iid = c.st().item.as_ref().map(|i| i.id);
                c.load_project(pid, iid, true);
                let mut msg = format!("{done} arquivo(s) baixado(s).");
                if skipped > 0 {
                    msg.push_str(&format!(" {skipped} ignorado(s) por terem alterações locais."));
                }
                Ctx::info(ui, msg);
            },
        );
    }

    fn open_file(&self) {
        if let Some((_, local)) = self.current() {
            if let Err(e) = open::that_detached(&local.path) {
                self.with_ui(|ui| self.show_error(ui, &anyhow!("Não foi possível abrir o arquivo: {e}"), false));
            }
        }
    }

    fn open_folder(&self) {
        let dir = {
            let st = self.st();
            let ws = st.workspace.as_ref();
            let project = st.projects.iter().find(|p| Some(p.id) == st.project_id);
            match (ws, project) {
                (Some(ws), Some(p)) => Some(ws.project_dir(&p.name)),
                (Some(ws), None) => Some(ws.root().to_path_buf()),
                _ => None,
            }
        };
        if let Some(dir) = dir {
            let _ = std::fs::create_dir_all(&dir);
            let _ = open::that_detached(dir);
        }
    }

    fn save_version_as(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let row = ui.global::<AppState>().get_current_version_row();
        let (item, version) = {
            let st = self.st();
            let Some(item) = st.item.clone() else { return };
            let Some(v) = st.versions.get(row as usize).cloned() else { return };
            (item, v)
        };
        let ctx = self.clone();
        let file_name = export_name(&item.file_name, &item.project_kind, &version);
        dialogs::ask(Dialog::SaveFile { file_name }, move |dest| {
            ctx.run(
                "Baixando versão…",
                move |c| c.api()?.download(version.id, &dest).map(|_| dest),
                |_, ui, dest| Ctx::info(ui, format!("Salvo em {}", dest.display())),
            );
        });
    }

    fn set_version_status(&self, status: String) {
        let Some(ui) = self.ui.upgrade() else { return };
        let row = ui.global::<AppState>().get_current_version_row();
        let Some(v) = self.st().versions.get(row as usize).cloned() else { return };
        if v.status == status {
            return;
        }
        self.run(
            "Alterando status…",
            move |c| c.api()?.set_status(v.id, &status),
            |c, ui, _| {
                c.reload_current_item();
                c.refresh_reviews_quiet();
                Ctx::info(ui, "Status alterado.");
            },
        );
        // If the server refuses, the combo box must snap back: reload either way.
        self.reload_current_item_later();
    }

    fn reload_current_item_later(&self) {
        let c = self.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(800), move || c.reload_current_item());
    }

    fn refresh_reviews_quiet(&self) {
        self.run("", |c| c.api()?.reviews(), |c, ui, r| c.apply_reviews(ui, r));
    }

    // ------------------------------------------------------------------ baselines

    fn apply_baselines(&self, ui: &AppWindow, baselines: Vec<Baseline>) {
        let g = ui.global::<AppState>();
        g.set_baseline_list(list(baselines.iter().map(|b| b.name.clone()).collect()));
        g.set_baselines(model(
            baselines
                .iter()
                .map(|b| BaselineRow {
                    id: b.id as i32,
                    name: s(&b.name),
                    description: s(&b.description),
                    info: s(format!(
                        "{} item(ns) · {} · {}{}",
                        b.version_count,
                        b.created_by.as_ref().map(|u| u.name.as_str()).unwrap_or(""),
                        fmt::date(&b.created_at),
                        if b.description.is_empty() { String::new() } else { format!(" · {}", b.description) }
                    )),
                })
                .collect(),
        ));
        let idx = g.get_current_baseline_index();
        if idx >= baselines.len() as i32 {
            g.set_current_baseline_index(-1);
            g.set_baseline_entry_rows(rows(vec![]));
        }
        self.st().baselines = baselines;
    }

    fn create_baseline(&self, name: String, description: String, only_released: bool) {
        let Some(pid) = self.st().project_id else { return };
        self.run(
            "Congelando versões…",
            move |c| {
                let api = c.api()?;
                let detail = api.create_baseline(pid, &name, &description, only_released)?;
                Ok((detail, api.baselines(pid)?))
            },
            |c, ui, (detail, all)| {
                ui.global::<AppState>().set_dialog(s(""));
                c.apply_baselines(ui, all);
                let idx = c.st().baselines.iter().position(|b| b.id == detail.baseline.id);
                if let Some(i) = idx {
                    ui.global::<AppState>().set_current_baseline_index(i as i32);
                }
                c.apply_baseline_detail(ui, detail);
                Ctx::info(ui, "Congelamento criado.");
            },
        );
    }

    fn select_baseline(&self, index: i32) {
        let Some(id) = self.st().baselines.get(index as usize).map(|b| b.id) else { return };
        self.run("Carregando congelamento…", move |c| c.api()?.baseline(id), |c, ui, d| {
            c.apply_baseline_detail(ui, d);
            Ctx::info(ui, "");
        });
    }

    fn apply_baseline_detail(&self, ui: &AppWindow, d: BaselineDetail) {
        let labels = ui.global::<Labels>();
        let kind = {
            let st = self.st();
            st.projects.iter().find(|p| Some(p.id) == st.project_id).map(|p| p.kind.clone()).unwrap_or_default()
        };
        let table = d
            .entries
            .iter()
            .map(|e| {
                vec![
                    e.part_number.clone().unwrap_or_default(),
                    e.item_name.clone(),
                    fmt::version_label(&kind, &e.version.label),
                    labels.invoke_status(s(&e.version.status), s(&kind)).to_string(),
                    e.version.comment.clone(),
                ]
            })
            .collect();
        ui.global::<AppState>().set_baseline_entry_rows(rows(table));
        self.st().baseline = Some(d);
    }

    fn download_baseline(&self) {
        let (detail, project) = {
            let st = self.st();
            let Some(d) = st.baseline.clone() else { return };
            let p = st.projects.iter().find(|p| Some(p.id) == st.project_id).map(|p| p.name.clone()).unwrap_or_default();
            (d, p)
        };
        let ctx = self.clone();
        dialogs::ask(Dialog::PickFolder { title: "Pasta de destino" }, move |root| {
            {
                ctx.run(
                    "Baixando congelamento…",
                    move |c| {
                        let api = c.api()?;
                        let dir = root.join(workspace::sanitize(&format!("{project} - {}", detail.baseline.name)));
                        let mut manifest = format!("Congelamento: {}\nProjeto: {project}\n\n", detail.baseline.name);
                        for e in &detail.entries {
                            let dest = dir.join(workspace::sanitize(&e.item_name)).join(workspace::sanitize(&e.download_name));
                            api.download(e.version.id, &dest)?;
                            manifest.push_str(&format!("{}\t{}\t{}\t{}\n", e.item_name, e.version.label, e.version.status, e.version.sha256));
                        }
                        std::fs::write(dir.join("MANIFESTO.txt"), manifest)?;
                        Ok(dir)
                    },
                    |_, ui, dir| {
                        let _ = open::that_detached(&dir);
                        Ctx::info(ui, format!("Congelamento salvo em {}", dir.display()));
                    },
                );
            }
        });
    }

    // ------------------------------------------------------------------ admin

    fn load_users(&self) {
        self.run("Carregando usuários…", |c| c.api()?.users(), |c, ui, users| {
            c.apply_users(ui, users);
            Ctx::info(ui, "");
        });
    }

    fn apply_users(&self, ui: &AppWindow, users: Vec<User>) {
        let g = ui.global::<AppState>();
        let labels = ui.global::<Labels>();
        g.set_user_rows(rows(
            users
                .iter()
                .map(|u| {
                    vec![
                        u.name.clone(),
                        u.username.clone(),
                        labels.invoke_role(s(&u.role)).to_string(),
                        u.subsystem.clone().unwrap_or_default(),
                        if u.active { "Ativo".into() } else { "Desativado".into() },
                    ]
                })
                .collect(),
        ));
        g.set_users(model(
            users
                .iter()
                .map(|u| UserRow {
                    id: u.id as i32,
                    username: s(&u.username),
                    name: s(&u.name),
                    role: s(&u.role),
                    subsystem: s(u.subsystem.as_deref().unwrap_or("")),
                    active: u.active,
                })
                .collect(),
        ));
        if g.get_current_user_row() >= users.len() as i32 {
            g.set_current_user_row(-1);
        }
        self.st().users = users;
    }

    fn create_user(&self, username: String, name: String, password: String, role: String, subsystem: String) {
        self.run(
            "Criando usuário…",
            move |c| {
                let api = c.api()?;
                api.create_user(&username, &name, &password, &role, &subsystem)?;
                api.users()
            },
            |c, ui, users| {
                ui.global::<AppState>().set_dialog(s(""));
                c.apply_users(ui, users);
                Ctx::info(ui, "Usuário criado.");
            },
        );
    }

    fn selected_user(&self) -> Option<i64> {
        let ui = self.ui.upgrade()?;
        let row = ui.global::<AppState>().get_current_user_row();
        self.st().users.get(row as usize).map(|u| u.id)
    }

    fn patch_user(&self, patch: UserPatch, msg: &'static str) {
        let Some(id) = self.selected_user() else { return };
        self.run(
            "Salvando usuário…",
            move |c| {
                let api = c.api()?;
                api.patch_user(id, &patch)?;
                api.users()
            },
            move |c, ui, users| {
                ui.global::<AppState>().set_dialog(s(""));
                c.apply_users(ui, users);
                Ctx::info(ui, msg);
            },
        );
    }

    fn update_user(&self, role: String, subsystem: String, active: bool) {
        self.patch_user(UserPatch { role: Some(role), subsystem: Some(subsystem), active: Some(active), ..Default::default() }, "Usuário atualizado.");
    }

    fn reset_password(&self, password: String) {
        self.patch_user(UserPatch { password: Some(password), ..Default::default() }, "Senha redefinida. As sessões abertas desse usuário foram encerradas.");
    }

    // ------------------------------------------------------------------ settings

    fn choose_workspace(&self) {
        let ctx = self.clone();
        dialogs::ask(Dialog::PickFolder { title: "Pasta de trabalho" }, move |dir| ctx.set_workspace(dir));
    }

    fn set_workspace(&self, dir: PathBuf) {
        if dir.as_os_str().is_empty() || dir.is_relative() {
            self.with_ui(|ui| self.show_error(ui, &anyhow!("Informe um caminho completo para a pasta de trabalho."), false));
            return;
        }
        let url = self.st().api.as_ref().map(|a| a.base_url().to_owned()).unwrap_or_default();
        match Workspace::open(&dir, &url) {
            Ok(ws) => {
                {
                    let mut st = self.st();
                    st.workspace = Some(ws);
                    st.config.workspace = Some(dir.clone());
                    let _ = st.config.save();
                }
                self.with_ui(|ui| {
                    ui.global::<AppState>().set_workspace_path(s(dir.to_string_lossy()));
                    Ctx::info(ui, "Pasta de trabalho alterada.");
                });
                self.refresh(true);
            }
            Err(e) => self.with_ui(|ui| self.show_error(ui, &e, false)),
        }
    }

    fn set_open_after_checkout(&self, v: bool) {
        let mut st = self.st();
        st.config.open_after_checkout = v;
        let _ = st.config.save();
        drop(st);
        self.with_ui(|ui| ui.global::<AppState>().set_open_after_checkout(v));
    }

    fn change_password(&self, current: String, new: String) {
        self.run("Alterando senha…", move |c| c.api()?.change_password(&current, &new), |_, ui, _| {
            ui.global::<AppState>().set_dialog(s(""));
            Ctx::info(ui, "Senha alterada.");
        });
    }

    fn confirm_accepted(&self) {
        let action = self.st().pending_confirm.take();
        self.with_ui(|ui| ui.global::<AppState>().set_dialog(s("")));
        if let Some(a) = action {
            a(self);
        }
    }

    fn confirm_cancelled(&self) {
        self.st().pending_confirm = None;
        self.with_ui(|ui| ui.global::<AppState>().set_dialog(s("")));
    }
}

/// Position of a status key in `AppState.status-keys`.
fn status_index(key: &str) -> i32 {
    ["draft", "in_review", "approved", "released", "obsolete"].iter().position(|k| *k == key).unwrap_or(0) as i32
}

fn project_row(p: &Project) -> ProjectRow {
    ProjectRow {
        id: p.id as i32,
        name: s(&p.name),
        kind: s(&p.kind),
        subsystem: s(&p.subsystem),
        description: s(&p.description),
        require_lock: p.require_lock,
        archived: p.archived,
    }
}

fn activity_row(ui: &AppWindow, a: &api::Activity) -> ActivityRow {
    let labels = ui.global::<Labels>();
    let kind = a.project_kind.clone().unwrap_or_default();
    let mut text = format!(
        "{} {}",
        a.user.as_ref().map(|u| u.name.as_str()).unwrap_or("Sistema"),
        labels.invoke_action(s(&a.action))
    );
    if let Some(item) = &a.item_name {
        text.push_str(&format!(" {item}"));
    }
    if let Some(v) = &a.version_label {
        text.push_str(&format!(" {}", fmt::version_label(&kind, v)));
    }
    match (a.action.as_str(), &a.details) {
        ("status_changed", Some(d)) => {
            if let Some((from, to)) = d.split_once("->") {
                text.push_str(&format!(": {} → {}", labels.invoke_status(s(from), s(&kind)), labels.invoke_status(s(to), s(&kind))));
            }
        }
        ("version_published" | "item_created", _) => {}
        (_, Some(d)) if !d.is_empty() => text.push_str(&format!(" ({d})")),
        _ => {}
    }
    if let (Some(p), false) = (&a.project_name, a.action == "project_created") {
        text.push_str(&format!(" · {p}"));
    } else if let Some(p) = &a.project_name {
        text.push_str(&format!(" {p}"));
    }
    ActivityRow { date: s(fmt::date(&a.created_at)), text: s(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_names() {
        let v = Version {
            id: 1, item_id: 1, label: "B".into(), filename: "qualquer.dwg".into(), sha256: String::new(),
            size: 0, comment: String::new(), status: "draft".into(), author: None, created_at: String::new(), baselines: vec![],
        };
        assert_eq!(export_name("manga.dwg", "cad", &v), "manga_revB.dwg");
    }
}
