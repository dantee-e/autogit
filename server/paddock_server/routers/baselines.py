import os
import tempfile
import zipfile

from fastapi import APIRouter, Depends
from fastapi.responses import FileResponse
from sqlalchemy import select
from sqlalchemy.orm import Session
from starlette.background import BackgroundTask

from .. import services as svc
from ..deps import admin, current_user, get_db, get_store
from ..models import Baseline, Project, User
from ..schemas import BaselineCreateIn, BaselineDetail, BaselineOut, baseline_detail, baseline_out
from ..storage import BlobStore

router = APIRouter(prefix="/api", tags=["baselines"])


@router.get("/projects/{project_id}/baselines", response_model=list[BaselineOut])
def list_baselines(project_id: int, _: User = Depends(current_user), db: Session = Depends(get_db)):
    svc.get_or_404(db, Project, project_id, "Projeto")
    rows = db.scalars(select(Baseline).where(Baseline.project_id == project_id)
                      .order_by(Baseline.created_at.desc()))
    return [baseline_out(db, b) for b in rows]


@router.post("/projects/{project_id}/baselines", response_model=BaselineDetail, status_code=201)
def create_baseline(project_id: int, body: BaselineCreateIn, user: User = Depends(admin),
                    db: Session = Depends(get_db)):
    p = svc.get_or_404(db, Project, project_id, "Projeto")
    b = svc.create_baseline(db, user, p, body.name, body.description, body.only_released)
    return baseline_detail(db, b)


@router.get("/baselines/{baseline_id}", response_model=BaselineDetail)
def get_baseline(baseline_id: int, _: User = Depends(current_user), db: Session = Depends(get_db)):
    return baseline_detail(db, svc.get_or_404(db, Baseline, baseline_id, "Congelamento"))


@router.get("/baselines/{baseline_id}/zip")
def download_zip(baseline_id: int, _: User = Depends(current_user), db: Session = Depends(get_db),
                 store: BlobStore = Depends(get_store)):
    b = svc.get_or_404(db, Baseline, baseline_id, "Congelamento")
    detail = baseline_detail(db, b)
    project = db.get(Project, b.project_id)
    fd, tmp = tempfile.mkstemp(suffix=".zip")
    os.close(fd)
    lines = [f"Congelamento: {b.name}", f"Projeto: {project.name}", f"Criado em: {b.created_at.isoformat()}", ""]
    with zipfile.ZipFile(tmp, "w", zipfile.ZIP_DEFLATED, allowZip64=True) as z:
        for e in detail.entries:
            folder = svc.clean_filename(e.item_name) or f"item-{e.item_id}"
            z.write(store.path(e.version.sha256), f"{folder}/{e.download_name}")
            lines.append(f"{e.item_name}\t{e.version.label}\t{e.version.status}\t{e.version.sha256}")
        z.writestr("MANIFESTO.txt", "\n".join(lines) + "\n")
    name = f"{svc.clean_filename(project.name)}_{svc.clean_filename(b.name)}.zip"
    return FileResponse(tmp, filename=name, media_type="application/zip",
                        background=BackgroundTask(os.remove, tmp))
