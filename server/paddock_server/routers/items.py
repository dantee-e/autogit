from fastapi import APIRouter, Depends, File, Form, UploadFile
from fastapi.responses import FileResponse
from sqlalchemy import select
from sqlalchemy.orm import Session

from .. import services as svc
from ..deps import current_user, get_db, get_store, writer
from ..errors import NotFound
from ..models import Item, User, Version
from ..schemas import ItemOut, LockIn, StatusIn, VersionOut, item_out, version_out, versions_out
from ..storage import BlobStore
from ..versioning import DEFAULT_BUMP

router = APIRouter(prefix="/api", tags=["items"])


def _item(db: Session, item_id: int) -> Item:
    return svc.get_or_404(db, Item, item_id, "Item")


@router.get("/items/{item_id}", response_model=ItemOut)
def get_item(item_id: int, _: User = Depends(current_user), db: Session = Depends(get_db)):
    return item_out(db, _item(db, item_id))


@router.get("/items/{item_id}/versions", response_model=list[VersionOut])
def list_versions(item_id: int, _: User = Depends(current_user), db: Session = Depends(get_db)):
    _item(db, item_id)
    vs = list(db.scalars(select(Version).where(Version.item_id == item_id).order_by(Version.seq.desc())))
    return versions_out(db, vs)


@router.post("/items/{item_id}/versions", response_model=VersionOut, status_code=201)
def publish_version(item_id: int, comment: str = Form(...), bump: str | None = Form(None),
                    keep_lock: bool = Form(False), file: UploadFile = File(...),
                    user: User = Depends(writer), db: Session = Depends(get_db),
                    store: BlobStore = Depends(get_store)):
    item = _item(db, item_id)
    v = svc.add_version(db, store, user, item, file.file, file.filename or "", comment,
                        bump or DEFAULT_BUMP[item.project.kind], keep_lock)
    return versions_out(db, [v])[0]


@router.post("/items/{item_id}/lock", response_model=ItemOut)
def lock(item_id: int, body: LockIn, user: User = Depends(writer), db: Session = Depends(get_db)):
    item = _item(db, item_id)
    svc.lock_item(db, user, item, body.note)
    return item_out(db, item)


@router.delete("/items/{item_id}/lock", response_model=ItemOut)
def unlock(item_id: int, user: User = Depends(writer), db: Session = Depends(get_db)):
    item = _item(db, item_id)
    svc.unlock_item(db, user, item)
    return item_out(db, item)


@router.get("/items/{item_id}/verify")
def verify(item_id: int, _: User = Depends(current_user), db: Session = Depends(get_db),
           store: BlobStore = Depends(get_store)):
    """Recompute SHA-256 of every stored version of the item."""
    _item(db, item_id)
    vs = db.scalars(select(Version).where(Version.item_id == item_id))
    bad = [v.label for v in vs if not store.verify(v.sha256)]
    return {"ok": not bad, "corrupted": bad}


@router.patch("/versions/{version_id}", response_model=VersionOut)
def set_status(version_id: int, body: StatusIn, user: User = Depends(writer), db: Session = Depends(get_db)):
    v = svc.get_or_404(db, Version, version_id, "Versão")
    svc.set_status(db, user, v, body.status)
    return versions_out(db, [v])[0]


def _send(store: BlobStore, v: Version) -> FileResponse:
    path = store.path(v.sha256)
    if not path.exists():
        raise NotFound("O arquivo desta versão não foi encontrado no armazenamento.")
    item = v.item
    return FileResponse(path, filename=svc.download_name(item.project, item, v),
                        media_type="application/octet-stream",
                        headers={"X-Paddock-Version": v.label, "X-Paddock-Version-Id": str(v.id),
                                 "X-Paddock-SHA256": v.sha256})


@router.get("/versions/{version_id}/download")
def download(version_id: int, _: User = Depends(current_user), db: Session = Depends(get_db),
             store: BlobStore = Depends(get_store)):
    return _send(store, svc.get_or_404(db, Version, version_id, "Versão"))


@router.get("/items/{item_id}/versions/{label}/download")
def download_by_label(item_id: int, label: str, status: str | None = None,
                      _: User = Depends(current_user), db: Session = Depends(get_db),
                      store: BlobStore = Depends(get_store)):
    """`label` is a version label (1.2.0, B, A.1) or `latest`. Optional ?status=released."""
    _item(db, item_id)
    if label == "latest":
        v = svc.latest_version(db, item_id, status)
    else:
        v = db.scalar(select(Version).where(Version.item_id == item_id, Version.label == label))
    if v is None:
        raise NotFound("Versão não encontrada.")
    return _send(store, v)
