from datetime import timedelta

from fastapi import APIRouter, Depends, Request
from sqlalchemy import select
from sqlalchemy.orm import Session

from ..config import Settings, random_token
from ..deps import current_token, current_user, get_db, get_settings
from ..errors import Forbidden, Invalid, NotFound, TooManyRequests, Unauthorized
from ..models import ApiToken, User, utcnow
from ..schemas import (LoginIn, LoginOut, PasswordIn, TokenCreated, TokenCreateIn, TokenOut, UserOut,
                       user_out)
from ..security import MIN_PASSWORD, hash_password, hash_token, verify_password

router = APIRouter(prefix="/api/auth", tags=["auth"])


def _token_out(t: ApiToken, current_id: int | None) -> TokenOut:
    return TokenOut(id=t.id, name=t.name, created_at=t.created_at, last_used_at=t.last_used_at,
                    expires_at=t.expires_at, current=t.id == current_id)


@router.post("/login", response_model=LoginOut)
def login(body: LoginIn, request: Request, db: Session = Depends(get_db),
          settings: Settings = Depends(get_settings)):
    throttle = request.app.state.throttle
    username = body.username.strip().lower()
    key = f"{username}|{request.client.host if request.client else '-'}"
    if throttle.blocked(key):
        raise TooManyRequests("Muitas tentativas. Aguarde alguns minutos.")
    user = db.scalar(select(User).where(User.username == username, User.active.is_(True)))
    if user is None or not verify_password(body.password, user.password_hash):
        throttle.fail(key)
        raise Unauthorized("Usuário ou senha incorretos.")
    throttle.reset(key)
    raw = random_token()
    expires = utcnow() + timedelta(days=settings.token_days)
    db.add(ApiToken(user_id=user.id, token_hash=hash_token(raw), name=body.device_name.strip() or "Paddock",
                    expires_at=expires, last_used_at=utcnow()))
    db.commit()
    return LoginOut(token=raw, expires_at=expires, user=user_out(user))


@router.post("/logout", status_code=204)
def logout(token: ApiToken = Depends(current_token), db: Session = Depends(get_db)):
    db.delete(db.merge(token))
    db.commit()


@router.get("/me", response_model=UserOut)
def me(user: User = Depends(current_user)):
    return user_out(user)


@router.put("/password", status_code=204)
def change_password(body: PasswordIn, user: User = Depends(current_user), db: Session = Depends(get_db)):
    if not verify_password(body.current, user.password_hash):
        raise Forbidden("Senha atual incorreta.")
    if len(body.new) < MIN_PASSWORD:
        raise Invalid(f"A nova senha precisa ter pelo menos {MIN_PASSWORD} caracteres.")
    user.password_hash = hash_password(body.new)
    db.commit()


@router.get("/tokens", response_model=list[TokenOut])
def list_tokens(token: ApiToken = Depends(current_token), db: Session = Depends(get_db)):
    rows = db.scalars(select(ApiToken).where(ApiToken.user_id == token.user_id).order_by(ApiToken.id))
    return [_token_out(t, token.id) for t in rows]


@router.post("/tokens", response_model=TokenCreated, status_code=201)
def create_token(body: TokenCreateIn, token: ApiToken = Depends(current_token), db: Session = Depends(get_db)):
    """Long-lived token for scripts/CI (e.g. publishing compiled firmware). Never expires."""
    raw = random_token()
    t = ApiToken(user_id=token.user_id, token_hash=hash_token(raw), name=body.name.strip())
    db.add(t)
    db.commit()
    return TokenCreated(**_token_out(t, token.id).model_dump(), token=raw)


@router.delete("/tokens/{token_id}", status_code=204)
def revoke_token(token_id: int, token: ApiToken = Depends(current_token), db: Session = Depends(get_db)):
    t = db.get(ApiToken, token_id)
    if t is None or t.user_id != token.user_id:
        raise NotFound("Token não encontrado.")
    db.delete(t)
    db.commit()
