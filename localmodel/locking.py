from __future__ import annotations

import os
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .io import write_json_atomic


class LeaseBusyError(RuntimeError):
    pass


class FileLease:
    """A crash-safe cross-process byte lock with non-secret owner metadata."""

    def __init__(self, path: Path, owner: dict[str, Any]):
        self.path = path
        self.metadata_path = path.with_name(path.name + ".owner.json")
        self.owner = {
            **owner,
            "pid": os.getpid(),
            "lease_id": uuid.uuid4().hex,
            "acquired_at": datetime.now(timezone.utc).isoformat(),
        }
        self.file: Any = None

    def acquire(self) -> "FileLease":
        self.path.parent.mkdir(parents=True, exist_ok=True)
        handle: Any = None
        try:
            handle = self.path.open("a+b")
            handle.seek(0, os.SEEK_END)
            if handle.tell() == 0:
                handle.write(b" ")
                handle.flush()
            handle.seek(0)
            if os.name == "nt":
                import msvcrt

                msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl

                fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except (OSError, BlockingIOError) as exc:
            if handle is not None:
                handle.close()
            detail = ""
            try:
                detail = self.metadata_path.read_text(encoding="utf-8", errors="replace").strip()
            except OSError:
                pass
            raise LeaseBusyError(
                f"Lease is busy: {self.path}{': ' + detail if detail else ''}"
            ) from exc
        self.file = handle
        self._publish_owner()
        return self

    def release(self) -> None:
        if self.file is None:
            return
        handle, self.file = self.file, None
        try:
            try:
                self.metadata_path.unlink()
            except FileNotFoundError:
                pass
            handle.seek(0)
            if os.name == "nt":
                import msvcrt

                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        finally:
            handle.close()

    def update_owner(self, **fields: Any) -> None:
        if self.file is None:
            raise RuntimeError("cannot update an inactive lease")
        self.owner.update(fields)
        self._publish_owner()

    def _publish_owner(self) -> None:
        write_json_atomic(self.metadata_path, self.owner)

    def __enter__(self) -> "FileLease":
        return self.acquire()

    def __exit__(self, *_: object) -> None:
        self.release()
