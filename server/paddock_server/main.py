from fastapi import FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse

from . import __version__, db
from .config import Settings
from .errors import AppError
from .routers import auth, baselines, items, misc, projects, users
from .security import LoginThrottle
from .storage import BlobStore
from . import web


def create_app(settings: Settings | None = None) -> FastAPI:
    settings = settings or Settings.from_env()
    db.upgrade(settings.database_url)
    engine = db.make_engine(settings.database_url)

    app = FastAPI(title="Paddock", version=__version__,
                  description="Gestão de versões de CAD e software para equipes de Formula SAE.")
    app.state.settings = settings
    app.state.engine = engine
    app.state.sessionmaker = db.make_sessionmaker(engine)
    app.state.store = BlobStore(settings.storage_dir, settings.max_upload_bytes)
    app.state.throttle = LoginThrottle()

    @app.exception_handler(AppError)
    async def app_error(_req: Request, exc: AppError):
        return JSONResponse({"detail": exc.message}, status_code=exc.status_code)

    @app.exception_handler(RequestValidationError)
    async def validation_error(_req: Request, exc: RequestValidationError):
        fields = ", ".join(".".join(str(p) for p in e["loc"][1:]) or "corpo" for e in exc.errors())
        return JSONResponse({"detail": f"Dados inválidos: {fields}.", "errors": exc.errors()},
                            status_code=422)

    for r in (auth.router, users.router, projects.router, items.router, baselines.router, misc.router,
              web.router):
        app.include_router(r)
    return app
