from __future__ import annotations

import json
import os
import time
import uuid
from pathlib import Path
from typing import Any


def write_json_atomic(path: Path, value: dict[str, Any], *, sort_keys: bool = True) -> None:
    """Durably replace a JSON object without exposing a partial destination."""
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.{uuid.uuid4().hex}.tmp")
    try:
        with temporary.open("w", encoding="utf-8", newline="\n") as handle:
            json.dump(value, handle, indent=2, sort_keys=sort_keys)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        for attempt in range(200):
            try:
                os.replace(temporary, path)
                break
            except PermissionError:
                if attempt == 199:
                    raise
                time.sleep(0.01)
    finally:
        if temporary.exists():
            temporary.unlink()
