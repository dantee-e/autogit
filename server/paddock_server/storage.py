"""Content-addressed blob storage (SHA-256).

Each distinct file is written once to STORAGE_DIR/ab/abcdef...; versions with
identical content share a blob. Blobs are never modified or deleted, so a
published version is immutable.
"""
import hashlib
import os
import tempfile
from pathlib import Path
from typing import BinaryIO

CHUNK = 1024 * 1024


class TooLarge(Exception):
    pass


class BlobStore:
    def __init__(self, root: Path, max_bytes: int):
        self.root = Path(root)
        self.max_bytes = max_bytes
        self.root.mkdir(parents=True, exist_ok=True)

    def path(self, sha256: str) -> Path:
        return self.root / sha256[:2] / sha256

    def save(self, stream: BinaryIO) -> tuple[str, int]:
        """Store a stream; return (sha256, size)."""
        h = hashlib.sha256()
        size = 0
        fd, tmp = tempfile.mkstemp(dir=self.root, prefix=".upload-")
        try:
            with os.fdopen(fd, "wb") as out:
                while chunk := stream.read(CHUNK):
                    size += len(chunk)
                    if size > self.max_bytes:
                        raise TooLarge()
                    h.update(chunk)
                    out.write(chunk)
            digest = h.hexdigest()
            dest = self.path(digest)
            if dest.exists():
                os.remove(tmp)
            else:
                dest.parent.mkdir(parents=True, exist_ok=True)
                os.replace(tmp, dest)
            return digest, size
        except BaseException:
            if os.path.exists(tmp):
                os.remove(tmp)
            raise

    def verify(self, sha256: str) -> bool:
        p = self.path(sha256)
        if not p.exists():
            return False
        h = hashlib.sha256()
        with open(p, "rb") as f:
            while chunk := f.read(CHUNK):
                h.update(chunk)
        return h.hexdigest() == sha256
