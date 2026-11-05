from collections.abc import Iterator
from datetime import timedelta

from fastapi import Depends, Request
from sqlalchemy import select
from sqlalchemy.orm import Session

from .config import Settings
from .errors import Forbidden, Unauthorized
from .models import ApiToken, User, utcnow
from .security import hash_token
from .storage import BlobStore


def get_settings(request: Request) -> Settings:
    return request.app.state.settings


def get_store(request: Request) -> BlobStore:
    return request.app.state.store


def get_db(request: Request) -> Iterator[Session]:
    with request.app.state.sessionmaker() as session:
        yield session


def current_token(request: Request, db: Session = Depends(get_db)) -> ApiToken:
    header = request.headers.get("Authorization", "")
    if not header.startswith("Bearer "):
        raise Unauthorized("Faça login para continuar.")
    token = db.scalar(select(ApiToken).where(ApiToken.token_hash == hash_token(header[7:].strip())))
    now = utcnow()
    if token is None or not token.user.active or (token.expires_at and token.expires_at < now):
        raise Unauthorized("Sessão expirada ou inválida. Faça login novamente.")
    # Avoid a write on every request: refresh at most once a minute.
    if token.last_used_at is None or now - token.last_used_at > timedelta(minutes=1):
        token.last_used_at = now
        db.commit()
    return token


def current_user(token: ApiToken = Depends(current_token)) -> User:
    return token.user


def writer(user: User = Depends(current_user)) -> User:
    if user.role not in ("admin", "member"):
        raise Forbidden("Seu papel permite apenas visualizar e baixar.")
    return user


def admin(user: User = Depends(current_user)) -> User:
    if user.role != "admin":
        raise Forbidden("Apenas administradores podem fazer isso.")
    return user
