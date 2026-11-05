"""Minimal HTML pages: first-run setup and client downloads. Everything else is the API."""
import html
from urllib.parse import quote

from fastapi import APIRouter, Depends, Form
from fastapi.responses import FileResponse, HTMLResponse, RedirectResponse
from sqlalchemy import select
from sqlalchemy.orm import Session

from . import __version__
from .config import Settings
from .deps import get_db, get_settings
from .errors import AppError, NotFound
from .models import User
from .routers.users import insert_user, validate_new_user
from .schemas import UserCreateIn
from .services import log

router = APIRouter(include_in_schema=False)

STYLE = """
:root{--bg:#f4f5f7;--card:#fff;--ink:#1c1f24;--muted:#6b7280;--line:#e2e5ea;--accent:#d7261e}
@media (prefers-color-scheme:dark){:root{--bg:#111317;--card:#1a1d22;--ink:#e8eaed;--muted:#9aa1ab;--line:#2c3038}}
body{margin:0;font:15px/1.5 system-ui,sans-serif;background:var(--bg);color:var(--ink)}
main{max-width:560px;margin:3rem auto;padding:0 16px}
.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:1.5rem}
h1{margin:0 0 .3rem;font-size:1.5rem} p{color:var(--muted)}
form{display:flex;flex-direction:column;gap:.8rem}
label{display:flex;flex-direction:column;gap:.25rem;font-weight:500}
input{font:inherit;padding:.5rem .6rem;border:1px solid var(--line);border-radius:6px;background:var(--card);color:var(--ink)}
button{font:inherit;padding:.6rem;border:0;border-radius:6px;background:var(--accent);color:#fff;cursor:pointer}
.err{background:#fdecea;color:#b42318;padding:.6rem .8rem;border-radius:6px}
ul{padding-left:1.2rem} a{color:inherit}
small{color:var(--muted)}
"""


def page(title: str, body: str) -> HTMLResponse:
    return HTMLResponse(f"""<!doctype html><html lang="pt-BR"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>{html.escape(title)}</title>
<style>{STYLE}</style></head><body><main><div class="card">{body}</div>
<p><small>Paddock server {__version__} · <a href="/docs">API</a></small></p></main></body></html>""")


def _has_users(db: Session) -> bool:
    return db.scalar(select(User.id).limit(1)) is not None


def setup_form(error: str = "", name: str = "", username: str = "") -> HTMLResponse:
    err = f'<div class="err">{html.escape(error)}</div>' if error else ""
    return page("Configuração inicial", f"""
<h1>Configuração inicial</h1>
<p>Crie a conta do primeiro administrador (capitão ou líder técnico). O restante da equipe
é cadastrado depois, pelo aplicativo Paddock.</p>{err}
<form method="post" action="/setup">
<label>Nome completo <input name="name" required value="{html.escape(name)}"></label>
<label>Usuário <input name="username" required autocomplete="username" value="{html.escape(username)}"></label>
<label>Senha <input type="password" name="password" required minlength="8" autocomplete="new-password"></label>
<button>Criar administrador</button></form>""")


@router.get("/")
def index(db: Session = Depends(get_db), settings: Settings = Depends(get_settings)):
    if not _has_users(db):
        return setup_form()
    files = sorted(p for p in settings.downloads_dir.iterdir() if p.is_file() and not p.name.startswith("."))
    links = "".join(f'<li><a href="/downloads/{quote(p.name)}">{html.escape(p.name)}</a> '
                    f'<small>{p.stat().st_size / 1e6:.1f} MB</small></li>' for p in files)
    downloads = f"<ul>{links}</ul>" if links else (
        "<p>Nenhum instalador publicado ainda. Coloque os binários do aplicativo na pasta "
        f"<code>{html.escape(str(settings.downloads_dir))}</code>.</p>")
    return page(settings.team_name, f"""
<h1>Paddock · {html.escape(settings.team_name)}</h1>
<p>Este é o servidor de versões da equipe. Para usar, instale o aplicativo Paddock e conecte
neste endereço.</p><h3>Baixar o aplicativo</h3>{downloads}""")


@router.post("/setup")
def setup(name: str = Form(""), username: str = Form(""), password: str = Form(""),
          db: Session = Depends(get_db)):
    if _has_users(db):
        return RedirectResponse("/", status_code=303)
    body = UserCreateIn(username=username, name=name, password=password, role="admin")
    try:
        validate_new_user(db, body)
    except AppError as e:
        return setup_form(e.message, name, username)
    u = insert_user(db, body)
    log(db, "user_created", u, details=f"{u.username} (admin inicial)")
    db.commit()
    return page("Pronto", f"""<h1>Administrador criado</h1>
<p>Abra o aplicativo Paddock e entre como <b>{html.escape(u.username)}</b>.</p>
<p><a href="/">Voltar</a></p>""")


@router.get("/downloads/{filename}")
def client_download(filename: str, settings: Settings = Depends(get_settings)):
    root = settings.downloads_dir.resolve()
    path = (root / filename).resolve()
    if path.parent != root or not path.is_file():
        raise NotFound("Arquivo não encontrado.")
    return FileResponse(path, filename=path.name)
