"""Command line: `python -m paddock_server serve` / `create-admin`."""
import argparse
import getpass
import os
import sys


def main() -> None:
    ap = argparse.ArgumentParser(prog="paddock-server")
    sub = ap.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("serve", help="run the HTTP server")
    s.add_argument("--host", default=os.environ.get("PADDOCK_HOST", "0.0.0.0"))
    s.add_argument("--port", type=int, default=int(os.environ.get("PADDOCK_PORT", "8000")))
    s.add_argument("--reload", action="store_true", help="development auto-reload")
    c = sub.add_parser("create-admin", help="create an administrator from the terminal")
    c.add_argument("username")
    c.add_argument("--name")
    args = ap.parse_args()

    if args.cmd == "serve":
        import uvicorn
        uvicorn.run("paddock_server.main:create_app", factory=True, host=args.host, port=args.port,
                    reload=args.reload, proxy_headers=True)
        return

    from .config import Settings
    from .db import make_engine, make_sessionmaker, upgrade
    from .errors import AppError
    from .routers.users import insert_user, validate_new_user
    from .schemas import UserCreateIn

    settings = Settings.from_env()
    upgrade(settings.database_url)
    password = getpass.getpass("Senha: ")
    body = UserCreateIn(username=args.username, name=args.name or args.username, password=password, role="admin")
    with make_sessionmaker(make_engine(settings.database_url))() as db:
        try:
            validate_new_user(db, body)
        except AppError as e:
            sys.exit(e.message)
        insert_user(db, body)
        db.commit()
    print(f"Administrador '{args.username}' criado.")


if __name__ == "__main__":
    main()
