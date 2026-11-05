from fastapi import APIRouter, Depends, File, Form, UploadFile
from sqlalchemy import select
from sqlalchemy.orm import Session

from .. import services as svc
from ..deps import admin, current_user, get_db, get_store, writer
from ..models import Item, Project, User
from ..schemas import ItemOut, ProjectCreateIn, ProjectOut, ProjectPatchIn, item_out, project_out
from ..storage import BlobStore

router = APIRouter(prefix="/api/projects", tags=["projects"])


@router.get("", response_model=list[ProjectOut])
def list_projects(include_archived: bool = False, _: User = Depends(current_user),
                  db: Session = Depends(get_db)):
    q = select(Project).order_by(Project.archived, Project.subsystem, Project.name)
    if not include_archived:
        q = q.where(Project.archived.is_(False))
    return [project_out(db, p) for p in db.scalars(q)]


@router.post("", response_model=ProjectOut, status_code=201)
def create_project(body: ProjectCreateIn, user: User = Depends(writer), db: Session = Depends(get_db)):
    require_lock = body.require_lock if body.require_lock is not None else body.kind == "cad"
    p = svc.create_project(db, user, body.name, body.kind, body.subsystem, body.description, require_lock)
    return project_out(db, p)


@router.get("/{project_id}", response_model=ProjectOut)
def get_project(project_id: int, _: User = Depends(current_user), db: Session = Depends(get_db)):
    return project_out(db, svc.get_or_404(db, Project, project_id, "Projeto"))


@router.patch("/{project_id}", response_model=ProjectOut)
def patch_project(project_id: int, body: ProjectPatchIn, user: User = Depends(admin),
                  db: Session = Depends(get_db)):
    p = svc.get_or_404(db, Project, project_id, "Projeto")
    if body.description is not None:
        p.description = body.description.strip()
    if body.require_lock is not None:
        p.require_lock = body.require_lock
    if body.archived is not None and body.archived != p.archived:
        p.archived = body.archived
        svc.log(db, "project_archived" if p.archived else "project_restored", user, p.id, details=p.name)
    db.commit()
    return project_out(db, p)


@router.get("/{project_id}/items", response_model=list[ItemOut])
def list_items(project_id: int, _: User = Depends(current_user), db: Session = Depends(get_db)):
    svc.get_or_404(db, Project, project_id, "Projeto")
    items = db.scalars(select(Item).where(Item.project_id == project_id))
    items = sorted(items, key=lambda i: (i.part_number is None, i.part_number or "", i.name.lower()))
    return [item_out(db, i) for i in items]


@router.post("/{project_id}/items", response_model=ItemOut, status_code=201)
def create_item(project_id: int,
                name: str = Form(...), comment: str = Form(...), bump: str | None = Form(None),
                part_number: str | None = Form(None), description: str | None = Form(None),
                file: UploadFile = File(...),
                user: User = Depends(writer), db: Session = Depends(get_db),
                store: BlobStore = Depends(get_store)):
    """Create an item and upload its first version in one request."""
    from ..versioning import DEFAULT_BUMP
    p = svc.get_or_404(db, Project, project_id, "Projeto")
    item, _ = svc.create_item(db, store, user, p, name, part_number, description, file.file,
                              file.filename or "", comment, bump or DEFAULT_BUMP[p.kind])
    return item_out(db, item)
