# -*- mode: python ; coding: utf-8 -*-
from pathlib import Path

repo = Path(SPECPATH)
golden_files = [
    (str(path), str(path.parent.relative_to(repo)))
    for path in sorted((repo / "golden").rglob("*"))
    if path.is_file()
]

a = Analysis(
    [str(repo / "sidecar.py")],
    pathex=[str(repo)],
    binaries=[],
    datas=golden_files,
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name="ferry-engine",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    console=True,
)
