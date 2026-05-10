#!/usr/bin/env python3
"""DETCP dev check: verify repo layout and required docs (no network)."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

REQUIRED_DOCS = (
    ROOT / "README.md",
    ROOT / "docs" / "readme.md",
    ROOT / "docs" / "architecture.md",
    ROOT / "docs" / "implementation.md",
)

OPTIONAL_RUNTIME = (
    ("deploy/docker-compose.yml", ROOT / "deploy" / "docker-compose.yml"),
    ("edge/", ROOT / "edge"),
    ("cloud-plane/", ROOT / "cloud-plane"),
    ("proto/detcp/v1/telemetry.proto", ROOT / "proto" / "detcp" / "v1" / "telemetry.proto"),
    ("scripts/simulate_robot_fleet.py", ROOT / "scripts" / "simulate_robot_fleet.py"),
)


def main() -> int:
    missing_docs = [str(p.relative_to(ROOT)) for p in REQUIRED_DOCS if not p.is_file()]
    if missing_docs:
        print("error: missing required files:", ", ".join(missing_docs), file=sys.stderr)
        return 1

    print("DETCP dev check — OK")
    print("  required documentation: present")

    present = 0
    for label, path in OPTIONAL_RUNTIME:
        if path.exists():
            present += 1
            print(f"  optional [{label}]: present")
        else:
            print(f"  optional [{label}]: not yet in repo")

    if present == len(OPTIONAL_RUNTIME):
        print("  full stack layout detected. Try: make up && pip install -r scripts/requirements.txt && python3 scripts/simulate_robot_fleet.py")
    return 0


if __name__ == "__main__":
    sys.exit(main())
