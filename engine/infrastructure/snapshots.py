"""Filesystem location shared by native session snapshot implementations."""

from pathlib import Path


BACKUP_DIR = Path.home() / ".resume-harness" / "backups"
