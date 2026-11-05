from fastapi import APIRouter, Depends, Query
from sqlalchemy import or_, select
from sqlalchemy.orm import Session

from .. import __version__, versioning
from ..config import Settings
from ..deps import current_user, get_db, get_settings
from ..models import Activity, Item, Project, User, Version
from ..schemas import (ActivityOut, MetaOut, SearchOut, VersionWithContext, activity_out, item_out,
                       version_ctx)
from ..security import MIN_PASSWORD

router = APIRouter(prefix="/api", tags=["misc"])


@router.get("/meta", response_model=MetaOut)
def meta(settings: Settings = Depends(get_settings), db: Session = Depends(get_db)):
    """Public: lets the client check the server before login."""
    return MetaOut(team_name=settings.team_name, server_version=__version__,
                   subsystems=settings.subsystems, statuses=list(versioning.STATUSES),
                   bumps={k: list(v) for k, v in versioning.BUMPS.items()}, min_password=MIN_PASSWORD,
                   has_users=db.scalar(select(User.id).limit(1)) is not None)


@router.get("/activity", response_model=list[ActivityOut])
def activity(project_id: int | None = None, item_id: int | None = None,
             limit: int = Query(50, ge=1, le=500), _: User = Depends(current_user),
             db: Session = Depends(get_db)):
    q = select(Activity).order_by(Activity.id.desc()).limit(limit)
    if project_id:
        q = q.where(Activity.project_id == project_id)
    if item_id:
        q = q.where(Activity.item_id == item_id)
    return [activity_out(a) for a in db.scalars(q)]


@router.get("/reviews", response_model=list[VersionWithContext])
def reviews(_: User = Depends(current_user), db: Session = Depends(get_db)):
    """Versions waiting for review in active projects."""
    q = (select(Version).join(Item).join(Project)
         .where(Version.status == "in_review", Project.archived.is_(False)).order_by(Version.created_at))
    return [version_ctx(db, v) for v in db.scalars(q)]


@router.get("/locks", response_model=list[int])
def my_locks(user: User = Depends(current_user), db: Session = Depends(get_db)):
    """IDs of items currently checked out by the caller."""
    return list(db.scalars(select(Item.id).where(Item.locked_by_id == user.id)))


@router.get("/search", response_model=SearchOut)
def search(q: str = Query(..., min_length=1), _: User = Depends(current_user), db: Session = Depends(get_db)):
    like = f"%{q.strip()}%"
    items = db.scalars(select(Item).where(or_(Item.name.ilike(like), Item.part_number.ilike(like),
                                              Item.description.ilike(like))).limit(100))
    versions = db.scalars(select(Version).where(or_(Version.comment.ilike(like), Version.filename.ilike(like),
                                                    Version.sha256 == q.strip().lower()))
                          .order_by(Version.created_at.desc()).limit(100))
    return SearchOut(items=[item_out(db, i) for i in items], versions=[version_ctx(db, v) for v in versions])
