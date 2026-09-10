"""Read the application version and reject mismatched version tags."""

import os
from pathlib import Path
import re
import tomllib


def version():
    value = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise SystemExit(f"Packaging requires a stable version, got {value!r}")
    ref = os.environ.get("GITHUB_REF", "")
    if ref.startswith("refs/tags/") and ref != f"refs/tags/v{value}":
        raise SystemExit(f"Release tag {ref!r} does not match Cargo version {value}")
    return value


if __name__ == "__main__":
    print(version())
