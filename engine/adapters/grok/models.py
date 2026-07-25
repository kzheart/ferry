"""Grok model catalog."""
import subprocess

from ...system import executables

FALLBACK = [
    {"id": "grok-code-fast-1", "label": "grok-code-fast-1",
     "source": "fallback"},
]


def discover():
    result = subprocess.run(
        executables.argv("grok", "models"), capture_output=True,
        text=True, timeout=15, **executables.RUN_FLAGS,
    )
    if result.returncode:
        raise RuntimeError((result.stderr or "grok models failed")[:400])
    rows = []
    for line in result.stdout.splitlines():
        value = line.strip().split()[0] if line.strip() else ""
        if value and value.lower() not in {"model", "models"}:
            rows.append({"id": value, "label": line.strip(), "source": "cli"})
    return rows, "cli", rows[0]["id"] if rows else None


def fallback():
    return [dict(row) for row in FALLBACK]
