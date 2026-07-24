"""运行时只读资源定位，兼容源码目录与 PyInstaller bundle。"""
import sys
from pathlib import Path


def resource_root() -> Path:
    bundle_root = getattr(sys, "_MEIPASS", None)
    if bundle_root:
        return Path(bundle_root)
    return Path(__file__).resolve().parents[2]


def resource_path(*parts: str) -> Path:
    return resource_root().joinpath(*parts)
