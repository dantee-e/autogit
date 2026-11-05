"""Pydantic request/response models and ORM → schema serializers."""
from datetime import datetime

from pydantic import BaseModel, Field
from sqlalchemy import func, select
from sqlalchemy.orm import Session

from . import services, versioning
from .models import Activity, Baseline, Item, Project, User, Version, baseline_versions


class UserRef(BaseModel):
    id: int
    name: str


class UserOut(BaseModel):
    id: int
    username: str
    name: str
    role: str
    subsystem: str | None
    active: bool
    created_at: datetime


class LoginIn(BaseModel):
    username: str
    password: str
    device_name: str = Field("Paddock desktop", max_length=120)


class LoginOut(BaseModel):
    token: str
    expires_at: datetime | None
    user: UserOut


class TokenOut(BaseModel):
    id: int
    name: str
    created_at: datetime
    last_used_at: datetime | None
    expires_at: datetime | None
    current: bool


class TokenCreateIn(BaseModel):
    name: str = Field(min_length=1, max_length=120)


class TokenCreated(TokenOut):
    token: str


class PasswordIn(BaseModel):
    current: str
    new: str


class UserCreateIn(BaseModel):
    username: str
    name: str
    password: str
    role: str = "member"
    subsystem: str | None = None


class UserPatchIn(BaseModel):
    name: str | None = None
    role: str | None = None
    subsystem: str | None = None
    active: bool | None = None
    password: str | None = None


class ProjectOut(BaseModel):
    id: int
    name: str
    kind: str
    subsystem: str
    description: str
    require_lock: bool
    archived: bool
    item_count: int
    last_update: datetime | None
    created_at: datetime


class ProjectCreateIn(BaseModel):
    name: str
    kind: str
    subsystem: str
    description: str = ""
    require_lock: bool | None = None  # default: True for CAD, False for software


class ProjectPatchIn(BaseModel):
    description: str | None = None
    require_lock: bool | None = None
    archived: bool | None = None


class BaselineRef(BaseModel):
    id: int
    name: str


class VersionOut(BaseModel):
    id: int
    item_id: int
    seq: int
    label: str
    filename: str
    sha256: str
    size: int
    comment: str
    status: str
    author: UserRef | None
    created_at: datetime
    baselines: list[BaselineRef] = []


class ItemOut(BaseModel):
    id: int
    project_id: int
    project_name: str
    project_kind: str
    require_lock: bool
    name: str
    part_number: str | None
    description: str
    file_name: str
    locked_by: UserRef | None
    locked_at: datetime | None
    lock_note: str | None
    latest_version: VersionOut | None
    version_count: int
    next_labels: dict[str, str]


class LockIn(BaseModel):
    note: str | None = None


class StatusIn(BaseModel):
    status: str


class BaselineOut(BaseModel):
    id: int
    project_id: int
    name: str
    description: str
    created_by: UserRef | None
    created_at: datetime
    version_count: int


class BaselineEntry(BaseModel):
    item_id: int
    item_name: str
    part_number: str | None
    file_name: str
    download_name: str
    version: VersionOut


class BaselineDetail(BaselineOut):
    entries: list[BaselineEntry]


class BaselineCreateIn(BaseModel):
    name: str
    description: str = ""
    only_released: bool = False


class ActivityOut(BaseModel):
    id: int
    action: str
    user: UserRef | None
    project_id: int | None
    project_name: str | None
    project_kind: str | None
    item_id: int | None
    item_name: str | None
    version_id: int | None
    version_label: str | None
    details: str | None
    created_at: datetime


class VersionWithContext(VersionOut):
    item_name: str
    project_id: int
    project_name: str
    project_kind: str


class SearchOut(BaseModel):
    items: list[ItemOut]
    versions: list[VersionWithContext]


class MetaOut(BaseModel):
    team_name: str
    server_version: str
    subsystems: list[str]
    statuses: list[str]
    bumps: dict[str, list[str]]
    min_password: int
    has_users: bool


# ------------------------------------------------------------------ serializers

def user_ref(u: User | None) -> UserRef | None:
    return UserRef(id=u.id, name=u.name) if u else None


def user_out(u: User) -> UserOut:
    return UserOut(id=u.id, username=u.username, name=u.name, role=u.role, subsystem=u.subsystem,
                   active=u.active, created_at=u.created_at)


def project_out(db: Session, p: Project) -> ProjectOut:
    item_count = db.scalar(select(func.count()).select_from(Item).where(Item.project_id == p.id))
    last = db.scalar(select(func.max(Version.created_at)).join(Item).where(Item.project_id == p.id))
    if last is not None and last.tzinfo is None:
        from datetime import timezone
        last = last.replace(tzinfo=timezone.utc)
    return ProjectOut(id=p.id, name=p.name, kind=p.kind, subsystem=p.subsystem, description=p.description,
                      require_lock=p.require_lock, archived=p.archived, item_count=item_count or 0,
                      last_update=last, created_at=p.created_at)


def _baselines_for(db: Session, version_ids: list[int]) -> dict[int, list[BaselineRef]]:
    out: dict[int, list[BaselineRef]] = {}
    if not version_ids:
        return out
    rows = db.execute(select(baseline_versions.c.version_id, Baseline.id, Baseline.name)
                      .join(Baseline, Baseline.id == baseline_versions.c.baseline_id)
                      .where(baseline_versions.c.version_id.in_(version_ids)))
    for vid, bid, name in rows:
        out.setdefault(vid, []).append(BaselineRef(id=bid, name=name))
    return out


def version_out(v: Version, baselines: list[BaselineRef] | None = None) -> VersionOut:
    return VersionOut(id=v.id, item_id=v.item_id, seq=v.seq, label=v.label, filename=v.filename,
                      sha256=v.sha256, size=v.size, comment=v.comment, status=v.status,
                      author=user_ref(v.author), created_at=v.created_at, baselines=baselines or [])


def versions_out(db: Session, versions: list[Version]) -> list[VersionOut]:
    bl = _baselines_for(db, [v.id for v in versions])
    return [version_out(v, bl.get(v.id)) for v in versions]


def version_ctx(db: Session, v: Version) -> VersionWithContext:
    base = versions_out(db, [v])[0]
    return VersionWithContext(**base.model_dump(), item_name=v.item.name, project_id=v.item.project_id,
                              project_name=v.item.project.name, project_kind=v.item.project.kind)


def item_out(db: Session, item: Item) -> ItemOut:
    latest = services.latest_version(db, item.id)
    count = db.scalar(select(func.count()).select_from(Version).where(Version.item_id == item.id))
    p = item.project
    return ItemOut(
        id=item.id, project_id=p.id, project_name=p.name, project_kind=p.kind, require_lock=p.require_lock,
        name=item.name, part_number=item.part_number, description=item.description,
        file_name=services.item_file_name(item, latest), locked_by=user_ref(item.locked_by),
        locked_at=item.locked_at, lock_note=item.lock_note,
        latest_version=versions_out(db, [latest])[0] if latest else None, version_count=count or 0,
        next_labels=versioning.previews(p.kind, latest.label if latest else None),
    )


def baseline_out(db: Session, b: Baseline) -> BaselineOut:
    n = db.scalar(select(func.count()).select_from(baseline_versions)
                  .where(baseline_versions.c.baseline_id == b.id))
    return BaselineOut(id=b.id, project_id=b.project_id, name=b.name, description=b.description,
                       created_by=user_ref(b.created_by), created_at=b.created_at, version_count=n or 0)


def baseline_detail(db: Session, b: Baseline) -> BaselineDetail:
    project = db.get(Project, b.project_id)
    vs = sorted(b.versions, key=lambda v: (v.item.part_number is None, v.item.part_number or "",
                                           v.item.name.lower()))
    outs = versions_out(db, vs)
    entries = [BaselineEntry(item_id=v.item_id, item_name=v.item.name, part_number=v.item.part_number,
                             file_name=services.item_file_name(v.item, v),
                             download_name=services.download_name(project, v.item, v), version=o)
               for v, o in zip(vs, outs)]
    return BaselineDetail(**baseline_out(db, b).model_dump(), entries=entries)


def activity_out(a: Activity) -> ActivityOut:
    return ActivityOut(id=a.id, action=a.action, user=user_ref(a.user), project_id=a.project_id,
                       project_name=a.project.name if a.project else None,
                       project_kind=a.project.kind if a.project else None,
                       item_id=a.item_id, item_name=a.item.name if a.item else None,
                       version_id=a.version_id, version_label=a.version.label if a.version else None,
                       details=a.details, created_at=a.created_at)
