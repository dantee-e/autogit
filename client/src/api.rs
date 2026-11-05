//! HTTP client for the Paddock server API.
//!
//! Blocking (reqwest::blocking); always called from background threads, never
//! from the UI thread.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response, multipart};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

// ------------------------------------------------------------------ models

#[derive(Debug, Clone, Deserialize)]
pub struct UserRef {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub name: String,
    pub role: String,
    pub subsystem: Option<String>,
    pub active: bool,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "admin"
    }
    pub fn can_write(&self) -> bool {
        self.role == "admin" || self.role == "member"
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginOut {
    pub token: String,
    pub user: User,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Meta {
    pub team_name: String,
    pub server_version: String,
    pub subsystems: Vec<String>,
    pub has_users: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub subsystem: String,
    pub description: String,
    pub require_lock: bool,
    pub archived: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BaselineRef {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Version {
    pub id: i64,
    pub item_id: i64,
    pub label: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
    pub comment: String,
    pub status: String,
    pub author: Option<UserRef>,
    pub created_at: String,
    #[serde(default)]
    pub baselines: Vec<BaselineRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionCtx {
    #[serde(flatten)]
    pub version: Version,
    pub item_name: String,
    pub project_id: i64,
    pub project_name: String,
    pub project_kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: i64,
    pub project_id: i64,
    pub project_name: String,
    pub project_kind: String,
    pub require_lock: bool,
    pub name: String,
    pub part_number: Option<String>,
    pub description: String,
    pub file_name: String,
    pub locked_by: Option<UserRef>,
    pub locked_at: Option<String>,
    pub lock_note: Option<String>,
    pub latest_version: Option<Version>,
    pub version_count: i64,
    pub next_labels: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Baseline {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub created_by: Option<UserRef>,
    pub created_at: String,
    pub version_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BaselineEntry {
    pub item_name: String,
    pub part_number: Option<String>,
    pub download_name: String,
    pub version: Version,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BaselineDetail {
    #[serde(flatten)]
    pub baseline: Baseline,
    pub entries: Vec<BaselineEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Activity {
    pub action: String,
    pub user: Option<UserRef>,
    pub project_name: Option<String>,
    pub project_kind: Option<String>,
    pub item_name: Option<String>,
    pub version_label: Option<String>,
    pub details: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UserPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

// ------------------------------------------------------------------ client

#[derive(Clone)]
pub struct Api {
    base: String,
    token: Option<String>,
    http: Client,
}

/// Server error message (`detail`), shown to the user as-is (server writes PT-BR).
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

const JSON_TIMEOUT: Duration = Duration::from_secs(30);

pub fn normalize_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("http://{url}")
    }
}

impl Api {
    pub fn new(base: &str, token: Option<String>) -> Result<Self> {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(None) // big CAD files: per-request timeouts are set for JSON calls only
            .user_agent(concat!("paddock-client/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { base: normalize_url(base), token, http })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    fn req(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        let mut r = self.http.request(method, format!("{}{}", self.base, path));
        if let Some(t) = &self.token {
            r = r.bearer_auth(t);
        }
        r
    }

    fn send(rb: RequestBuilder) -> Result<Response> {
        let resp = rb.send().map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                anyhow!("Não foi possível conectar ao servidor. Verifique o endereço e a rede.")
            } else {
                anyhow!("Erro de comunicação com o servidor: {e}")
            }
        })?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let message = resp
            .json::<serde_json::Value>()
            .ok()
            .and_then(|v| v.get("detail").and_then(|d| d.as_str()).map(str::to_owned))
            .unwrap_or_else(|| format!("O servidor respondeu com erro {status}."));
        Err(ApiError { status, message }.into())
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(Self::send(self.req(reqwest::Method::GET, path).timeout(JSON_TIMEOUT))?.json()?)
    }

    fn json<T: DeserializeOwned>(&self, method: reqwest::Method, path: &str, body: serde_json::Value) -> Result<T> {
        Ok(Self::send(self.req(method, path).timeout(JSON_TIMEOUT).json(&body))?.json()?)
    }

    // ---- auth
    pub fn meta(&self) -> Result<Meta> {
        self.get("/api/meta")
    }

    pub fn login(&mut self, username: &str, password: &str, device: &str) -> Result<LoginOut> {
        let out: LoginOut = self.json(
            reqwest::Method::POST,
            "/api/auth/login",
            json!({"username": username, "password": password, "device_name": device}),
        )?;
        self.token = Some(out.token.clone());
        Ok(out)
    }

    pub fn logout(&self) -> Result<()> {
        Self::send(self.req(reqwest::Method::POST, "/api/auth/logout").timeout(JSON_TIMEOUT))?;
        Ok(())
    }

    pub fn me(&self) -> Result<User> {
        self.get("/api/auth/me")
    }

    pub fn change_password(&self, current: &str, new: &str) -> Result<()> {
        Self::send(
            self.req(reqwest::Method::PUT, "/api/auth/password")
                .timeout(JSON_TIMEOUT)
                .json(&json!({"current": current, "new": new})),
        )?;
        Ok(())
    }

    // ---- projects
    pub fn projects(&self, include_archived: bool) -> Result<Vec<Project>> {
        self.get(&format!("/api/projects?include_archived={include_archived}"))
    }

    pub fn create_project(&self, name: &str, kind: &str, subsystem: &str, description: &str, require_lock: bool) -> Result<Project> {
        self.json(
            reqwest::Method::POST,
            "/api/projects",
            json!({"name": name, "kind": kind, "subsystem": subsystem, "description": description,
                   "require_lock": require_lock}),
        )
    }

    pub fn patch_project(&self, id: i64, body: serde_json::Value) -> Result<Project> {
        self.json(reqwest::Method::PATCH, &format!("/api/projects/{id}"), body)
    }

    // ---- items
    pub fn items(&self, project_id: i64) -> Result<Vec<Item>> {
        self.get(&format!("/api/projects/{project_id}/items"))
    }

    pub fn item(&self, id: i64) -> Result<Item> {
        self.get(&format!("/api/items/{id}"))
    }

    pub fn versions(&self, item_id: i64) -> Result<Vec<Version>> {
        self.get(&format!("/api/items/{item_id}/versions"))
    }

    pub fn create_item(
        &self, project_id: i64, name: &str, part_number: &str, description: &str, comment: &str, bump: &str, file: &Path,
    ) -> Result<Item> {
        let form = multipart::Form::new()
            .text("name", name.to_owned())
            .text("part_number", part_number.to_owned())
            .text("description", description.to_owned())
            .text("comment", comment.to_owned())
            .text("bump", bump.to_owned())
            .part("file", file_part(file)?);
        Ok(Self::send(self.req(reqwest::Method::POST, &format!("/api/projects/{project_id}/items")).multipart(form))?.json()?)
    }

    pub fn publish(&self, item_id: i64, file: &Path, comment: &str, bump: &str, keep_lock: bool) -> Result<Version> {
        let form = multipart::Form::new()
            .text("comment", comment.to_owned())
            .text("bump", bump.to_owned())
            .text("keep_lock", keep_lock.to_string())
            .part("file", file_part(file)?);
        Ok(Self::send(self.req(reqwest::Method::POST, &format!("/api/items/{item_id}/versions")).multipart(form))?.json()?)
    }

    pub fn lock(&self, item_id: i64, note: &str) -> Result<Item> {
        self.json(reqwest::Method::POST, &format!("/api/items/{item_id}/lock"), json!({"note": note}))
    }

    pub fn unlock(&self, item_id: i64) -> Result<Item> {
        Ok(Self::send(self.req(reqwest::Method::DELETE, &format!("/api/items/{item_id}/lock")).timeout(JSON_TIMEOUT))?.json()?)
    }

    pub fn set_status(&self, version_id: i64, status: &str) -> Result<Version> {
        self.json(reqwest::Method::PATCH, &format!("/api/versions/{version_id}"), json!({"status": status}))
    }

    /// Stream a version to `dest` (via a temp file in the same folder).
    /// Returns the SHA-256 of what was written and checks it against the server's.
    pub fn download(&self, version_id: i64, dest: &Path) -> Result<String> {
        let mut resp = Self::send(self.req(reqwest::Method::GET, &format!("/api/versions/{version_id}/download")))?;
        let expected = resp
            .headers()
            .get("x-paddock-sha256")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let dir = dest.parent().context("destino inválido")?;
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(
            ".{}.paddock-tmp",
            dest.file_name().and_then(|n| n.to_str()).unwrap_or("download")
        ));
        let result = (|| -> Result<String> {
            let mut out = HashingWriter { inner: File::create(&tmp)?, hasher: Sha256::new() };
            resp.copy_to(&mut out)?;
            out.inner.sync_all()?;
            let digest = hex::encode(out.hasher.finalize());
            if let Some(exp) = &expected {
                if exp != &digest {
                    bail!("O download chegou corrompido (hash diferente). Tente novamente.");
                }
            }
            Ok(digest)
        })();
        match result {
            Ok(digest) => {
                crate::workspace::replace_file(&tmp, dest)?;
                Ok(digest)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    // ---- reviews, activity, locks
    pub fn reviews(&self) -> Result<Vec<VersionCtx>> {
        self.get("/api/reviews")
    }

    pub fn activity(&self, project_id: Option<i64>, item_id: Option<i64>, limit: u32) -> Result<Vec<Activity>> {
        let mut q = format!("/api/activity?limit={limit}");
        if let Some(p) = project_id {
            q.push_str(&format!("&project_id={p}"));
        }
        if let Some(i) = item_id {
            q.push_str(&format!("&item_id={i}"));
        }
        self.get(&q)
    }

    // ---- baselines
    pub fn baselines(&self, project_id: i64) -> Result<Vec<Baseline>> {
        self.get(&format!("/api/projects/{project_id}/baselines"))
    }

    pub fn baseline(&self, id: i64) -> Result<BaselineDetail> {
        self.get(&format!("/api/baselines/{id}"))
    }

    pub fn create_baseline(&self, project_id: i64, name: &str, description: &str, only_released: bool) -> Result<BaselineDetail> {
        self.json(
            reqwest::Method::POST,
            &format!("/api/projects/{project_id}/baselines"),
            json!({"name": name, "description": description, "only_released": only_released}),
        )
    }

    // ---- users (admin)
    pub fn users(&self) -> Result<Vec<User>> {
        self.get("/api/users")
    }

    pub fn create_user(&self, username: &str, name: &str, password: &str, role: &str, subsystem: &str) -> Result<User> {
        self.json(
            reqwest::Method::POST,
            "/api/users",
            json!({"username": username, "name": name, "password": password, "role": role,
                   "subsystem": if subsystem.is_empty() { None } else { Some(subsystem) }}),
        )
    }

    pub fn patch_user(&self, id: i64, patch: &UserPatch) -> Result<User> {
        self.json(reqwest::Method::PATCH, &format!("/api/users/{id}"), serde_json::to_value(patch)?)
    }
}

pub fn is_unauthorized(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiError>().is_some_and(|a| a.status == StatusCode::UNAUTHORIZED)
}

fn file_part(path: &Path) -> Result<multipart::Part> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("nome de arquivo inválido")?
        .to_owned();
    let file = File::open(path).with_context(|| format!("Não foi possível abrir {}", path.display()))?;
    let len = file.metadata()?.len();
    Ok(multipart::Part::reader_with_length(file, len).file_name(name))
}

struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
