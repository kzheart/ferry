# -*- mode: python ; coding: utf-8 -*-
from pathlib import Path
from PyInstaller.utils.hooks import collect_submodules

repo = Path(SPECPATH)

a = Analysis(
    [str(repo / "sidecar.py")],
    pathex=[str(repo)],
    binaries=[],
    datas=[],
    # Adapter packages are discovered with pkgutil at runtime. PyInstaller
    # cannot infer those imports from the source graph, so include them
    # explicitly in the sidecar bundle.
    hiddenimports=collect_submodules("engine.adapters"),
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
