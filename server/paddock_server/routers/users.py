import re

from fastapi import APIRouter, Depends
from sqlalchemy import delete, select, update
from sqlalchemy.orm import Session

from ..deps import admin, current_user, get_db
from ..errors import Conflict, Invalid, NotFound
from ..models import ApiToken, Item, User
from ..schemas import UserCreateIn, UserOut, UserPatchIn, UserRef, user_out
from ..security import MIN_PASSWORD, hash_password
from ..services import log

router = APIRouter(prefix="/api/users", tags=["users"])

ROLES = ("admin", "member", "viewer")
USERNAME_RE = re.compile(r"^[a-z0-9._-]{2,64}$")


def validate_new_user(db: Session, body: UserCreateIn) -> None:
    if not USERNAME_RE.match(body.username.strip().lower()):
        raise Invalid("Usuário deve ter 2 a 64 caracteres: letras, números, ponto, hífen ou sublinhado.")
    if not body.name.strip():
        raise Invalid("Informe o nome.")
    if len(body.password) < MIN_PASSWORD:
        raise Invalid(f"A senha precisa ter pelo menos {MIN_PASSWORD} caracteres.")
    if body.role not in ROLES:
        raise Invalid("Papel inválido.")
    if db.scalar(select(User.id).where(User.username == body.username.strip().lower())):
        raise Conflict("Esse nome de usuário já existe.")


def insert_user(db: Session, body: UserCreateIn) -> User:
    u = User(username=body.username.strip().lower(), name=body.name.strip(),
             password_hash=hash_password(body.password), role=body.role,
             subsystem=(body.subsystem or "").strip() or None)
    db.add(u)
    db.flush()
    return u


@router.get("/directory", response_model=list[UserRef])
def directory(_: User = Depends(current_user), db: Session = Depends(get_db)):
    """Names of active users, visible to everyone (for filters and display)."""
    return [UserRef(id=u.id, name=u.name)
            for u in db.scalars(select(User).where(User.active.is_(True)).order_by(User.name))]


@router.get("", response_model=list[UserOut])
def list_users(_: User = Depends(admin), db: Session = Depends(get_db)):
    return [user_out(u) for u in db.scalars(select(User).order_by(User.active.desc(), User.name))]


@router.post("", response_model=UserOut, status_code=201)
def create_user(body: UserCreateIn, me: User = Depends(admin), db: Session = Depends(get_db)):
    validate_new_user(db, body)
    u = insert_user(db, body)
    log(db, "user_created", me, details=f"{u.username} ({u.role})")
    db.commit()
    return user_out(u)


@router.patch("/{user_id}", response_model=UserOut)
def patch_user(user_id: int, body: UserPatchIn, me: User = Depends(admin), db: Session = Depends(get_db)):
    u = db.get(User, user_id)
    if u is None:
        raise NotFound("Usuário não encontrado.")
    if u.id == me.id and (body.role not in (None, me.role) or body.active is False):
        raise Invalid("Você não pode alterar o próprio papel nem se desativar.")
    if body.name is not None:
        if not body.name.strip():
            raise Invalid("Informe o nome.")
        u.name = body.name.strip()
    if body.role is not None:
        if body.role not in ROLES:
            raise Invalid("Papel inválido.")
        u.role = body.role
    if body.subsystem is not None:
        u.subsystem = body.subsystem.strip() or None
    if body.password is not None:
        if len(body.password) < MIN_PASSWORD:
            raise Invalid(f"A senha precisa ter pelo menos {MIN_PASSWORD} caracteres.")
        u.password_hash = hash_password(body.password)
        db.execute(delete(ApiToken).where(ApiToken.user_id == u.id))
    if body.active is not None and body.active != u.active:
        u.active = body.active
        if not body.active:
            # Someone leaving the team: revoke sessions and free their check-outs.
            db.execute(delete(ApiToken).where(ApiToken.user_id == u.id))
            db.execute(update(Item).where(Item.locked_by_id == u.id)
                       .values(locked_by_id=None, locked_at=None, lock_note=None))
            log(db, "user_deactivated", me, details=u.username)
    db.commit()
    return user_out(u)
