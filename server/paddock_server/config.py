"""Server configuration, read from environment variables."""
import os
import secrets
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_SUBSYSTEMS = [
    "Chassi", "Suspensão", "Direção", "Freios", "Powertrain", "Transmissão",
    "Aerodinâmica", "Eletrônica", "Telemetria", "Ergonomia", "Arrefecimento", "Geral",
]


@dataclass
class Settings:
    data_dir: Path
    database_url: str
    storage_dir: Path
    downloads_dir: Path
    team_name: str = "Equipe Formula SAE"
    max_upload_bytes: int = 1024 * 1024 * 1024
    token_days: int = 90
    subsystems: list[str] = field(default_factory=lambda: list(DEFAULT_SUBSYSTEMS))

    @classmethod
    def from_env(cls, **overrides) -> "Settings":
        data_dir = Path(overrides.pop("data_dir", None) or os.environ.get("PADDOCK_DATA", "./data")).resolve()
        subsystems = os.environ.get("PADDOCK_SUBSYSTEMS")
        values = dict(
            data_dir=data_dir,
            database_url=os.environ.get("PADDOCK_DATABASE_URL", f"sqlite:///{data_dir / 'paddock.sqlite3'}"),
            storage_dir=data_dir / "files",
            downloads_dir=data_dir / "downloads",
            team_name=os.environ.get("PADDOCK_TEAM_NAME", "Equipe Formula SAE"),
            max_upload_bytes=int(os.environ.get("PADDOCK_MAX_UPLOAD_MB", "1024")) * 1024 * 1024,
            token_days=int(os.environ.get("PADDOCK_TOKEN_DAYS", "90")),
        )
        if subsystems:
            values["subsystems"] = [s.strip() for s in subsystems.split(",") if s.strip()]
        values.update(overrides)
        settings = cls(**values)
        for d in (settings.data_dir, settings.storage_dir, settings.downloads_dir):
            d.mkdir(parents=True, exist_ok=True)
        return settings


def random_token(prefix: str = "pdk_") -> str:
    return prefix + secrets.token_urlsafe(32)
