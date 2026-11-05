"""Business rules shared by all routers."""
import os
import re
from typing import BinaryIO

from sqlalchemy import func, select, update
from sqlalchemy.exc import IntegrityError
from sqlalchemy.orm import Session

from . import versioning
from .errors import Conflict, Forbidden, Invalid, NotFound
from .models import Activity, Baseline, Item, Project, User, Version, utcnow
from .storage import BlobStore, TooLarge

WINDOWS_RESERVED = {"CON", "PRN", "AUX", "NUL", *(f"COM{i}" for i in range(1, 10)),
                    *(f"LPT{i}" for i in range(1, 10))}


def is_admin(user: User) -> bool:
    return user.role == "admin"


def clean_filename(name: str | None) -> str:
    """Make a name safe on both Windows and Linux file systems."""
    name = (name or "").replace("\\", "/").split("/")[-1]
    name = re.sub(r'[\x00-\x1f\x7f<>:"|?*]', "", name).strip().rstrip(". ")
    stem = name.split(".")[0].upper()
    if stem in WINDOWS_RESERVED:
        name = "_" + name
    return name[:200]


def item_file_name(item: Item, version: Version | None) -> str:
    """Stable working-copy name: <first version stem><current extension>."""
    if version is None:
        return item.file_stem or clean_filename(item.name)
    stem, ext = os.path.splitext(version.filename)
    return f"{item.file_stem or stem}{ext}"


def download_name(project: Project, item: Item, version: Version) -> str:
    stem, ext = os.path.splitext(item_file_name(item, version))
    tag = f"rev{version.label}" if project.kind == "cad" else f"v{version.label}"
    return f"{stem}_{tag}{ext}"


def log(db: Session, action: str, user: User | None, project_id=None, item_id=None,
        version_id=None, details=None) -> None:
    db.add(Activity(action=action, user_id=user.id if user else None, project_id=project_id,
                    item_id=item_id, version_id=version_id, details=details))


def get_or_404(db: Session, model, obj_id: int, what: str):
    obj = db.get(model, obj_id)
    if obj is None:
        raise NotFound(f"{what} não encontrado.")
    return obj


def latest_version(db: Session, item_id: int, status: str | None = None) -> Version | None:
    q = select(Version).where(Version.item_id == item_id)
    if status:
        q = q.where(Version.status == status)
    return db.scalar(q.order_by(Version.seq.desc()).limit(1))


# ------------------------------------------------------------------ projects

def create_project(db: Session, user: User, name: str, kind: str, subsystem: str,
                   description: str, require_lock: bool) -> Project:
    name, subsystem = (name or "").strip(), (subsystem or "").strip()
    if not name or not subsystem:
        raise Invalid("Nome e subsistema são obrigatórios.")
    if kind not in versioning.BUMPS:
        raise Invalid("Tipo de projeto inválido.")
    if db.scalar(select(Project.id).where(func.lower(Project.name) == name.lower())):
        raise Conflict("Já existe um projeto com esse nome.")
    p = Project(name=name, kind=kind, subsystem=subsystem, description=(description or "").strip(),
                require_lock=require_lock, created_by_id=user.id)
    db.add(p)
    db.flush()
    log(db, "project_created", user, p.id, details=name)
    db.commit()
    return p


# ------------------------------------------------------------------ items & versions

def create_item(db: Session, store: BlobStore, user: User, project: Project, name: str,
                part_number: str | None, description: str | None, stream: BinaryIO,
                filename: str, comment: str, bump: str) -> tuple[Item, Version]:
    """Create an item together with its first version (atomic)."""
    if project.archived:
        raise Forbidden("Projeto arquivado.")
    name = (name or "").strip()
    if not name:
        raise Invalid("O nome do item é obrigatório.")
    if db.scalar(select(Item.id).where(Item.project_id == project.id,
                                       func.lower(Item.name) == name.lower())):
        raise Conflict("Já existe um item com esse nome neste projeto.")
    # Working copies live side by side in one folder per project, so base file
    # names must be unique within a project (also keeps AutoCAD xrefs unambiguous).
    stem = os.path.splitext(clean_filename(filename))[0]
    if stem and db.scalar(select(Item.id).where(Item.project_id == project.id,
                                                func.lower(Item.file_stem) == stem.lower())):
        raise Conflict(f"Já existe um item com o arquivo “{stem}” neste projeto. Renomeie o arquivo.")
    item = Item(project_id=project.id, name=name, part_number=(part_number or "").strip() or None,
                description=(description or "").strip(), created_by_id=user.id)
    db.add(item)
    db.flush()
    log(db, "item_created", user, project.id, item.id, details=name)
    version = _publish(db, store, user, item, project, stream, filename, comment, bump, keep_lock=False)
    return item, version


def add_version(db: Session, store: BlobStore, user: User, item: Item, stream: BinaryIO,
                filename: str, comment: str, bump: str, keep_lock: bool) -> Version:
    project = item.project
    if project.archived:
        raise Forbidden("Projeto arquivado: não aceita novas versões.")
    if item.locked_by_id and item.locked_by_id != user.id:
        raise Forbidden(f"Este item está em check-out com {item.locked_by.name}.")
    if project.require_lock and item.locked_by_id != user.id:
        raise Forbidden("Este projeto exige check-out antes de publicar uma nova versão.")
    return _publish(db, store, user, item, project, stream, filename, comment, bump, keep_lock)


def _publish(db, store, user, item, project, stream, filename, comment, bump, keep_lock) -> Version:
    filename = clean_filename(filename)
    comment = (comment or "").strip()
    if not filename:
        db.rollback()
        raise Invalid("Selecione um arquivo.")
    if not comment:
        db.rollback()
        raise Invalid("Descreva o que mudou nesta versão.")
    last = latest_version(db, item.id)
    try:
        label = versioning.next_label(project.kind, last.label if last else None, bump)
    except ValueError:
        db.rollback()
        raise Invalid("Tipo de incremento inválido.")
    try:
        sha, size = store.save(stream)
    except TooLarge:
        db.rollback()
        raise Invalid("Arquivo maior que o limite do servidor.")
    if size == 0:
        db.rollback()
        raise Invalid("O arquivo está vazio.")
    if last and last.sha256 == sha:
        db.rollback()
        raise Conflict(f"O arquivo é idêntico à versão atual ({last.label}).")

    v = Version(item_id=item.id, seq=(last.seq if last else 0) + 1, label=label, filename=filename,
                sha256=sha, size=size, comment=comment, author_id=user.id)
    db.add(v)
    if not item.file_stem:
        item.file_stem = os.path.splitext(filename)[0]
    if not keep_lock and item.locked_by_id == user.id:
        item.locked_by_id = item.locked_at = item.lock_note = None
    try:
        db.flush()
    except IntegrityError:
        db.rollback()
        raise Conflict("Outra versão foi publicada ao mesmo tempo. Atualize e tente de novo.")
    log(db, "version_published", user, project.id, item.id, v.id, details=label)
    db.commit()
    return v


def lock_item(db: Session, user: User, item: Item, note: str | None) -> None:
    if item.project.archived:
        raise Forbidden("Projeto arquivado.")
    note = (note or "").strip()[:300] or None
    res = db.execute(update(Item).where(Item.id == item.id, Item.locked_by_id.is_(None))
                     .values(locked_by_id=user.id, locked_at=utcnow(), lock_note=note))
    if res.rowcount == 0:
        db.rollback()
        db.refresh(item)
        who = item.locked_by.name if item.locked_by else "outra pessoa"
        raise Conflict(f"O item já está em check-out com {who}.")
    log(db, "checkout", user, item.project_id, item.id, details=note)
    db.commit()
    db.refresh(item)


def unlock_item(db: Session, user: User, item: Item) -> None:
    if not item.locked_by_id:
        raise Invalid("O item não está em check-out.")
    forced = item.locked_by_id != user.id
    if forced and not is_admin(user):
        raise Forbidden("Só quem fez o check-out ou um administrador pode liberar.")
    previous = item.locked_by.name if item.locked_by else None
    item.locked_by_id = item.locked_at = item.lock_note = None
    log(db, "checkout_forced" if forced else "checkin", user, item.project_id, item.id,
        details=previous if forced else None)
    db.commit()
    db.refresh(item)


def set_status(db: Session, user: User, version: Version, status: str) -> None:
    if status not in versioning.STATUSES:
        raise Invalid("Status inválido.")
    if status == version.status:
        return
    privileged = status in versioning.PRIVILEGED_STATUSES or version.status in versioning.PRIVILEGED_STATUSES
    if privileged and not is_admin(user):
        raise Forbidden("Apenas administradores aprovam, liberam ou alteram versões aprovadas/liberadas.")
    if version.item.project.archived:
        raise Forbidden("Projeto arquivado.")
    old = version.status
    version.status = status
    log(db, "status_changed", user, version.item.project_id, version.item_id, version.id,
        details=f"{old}->{status}")
    db.commit()


# ------------------------------------------------------------------ baselines

def create_baseline(db: Session, user: User, project: Project, name: str, description: str,
                    only_released: bool) -> Baseline:
    name = (name or "").strip()
    if not name:
        raise Invalid("Dê um nome ao congelamento.")
    if db.scalar(select(Baseline.id).where(Baseline.project_id == project.id, Baseline.name == name)):
        raise Conflict("Já existe um congelamento com esse nome.")
    chosen = []
    for item_id in db.scalars(select(Item.id).where(Item.project_id == project.id)):
        v = latest_version(db, item_id, "released" if only_released else None)
        if v:
            chosen.append(v)
    if not chosen:
        raise Invalid("Nenhuma versão elegível para congelar.")
    b = Baseline(project_id=project.id, name=name, description=(description or "").strip(),
                 created_by_id=user.id, versions=chosen)
    db.add(b)
    db.flush()
    log(db, "baseline_created", user, project.id, details=f"{name} ({len(chosen)})")
    db.commit()
    return b
