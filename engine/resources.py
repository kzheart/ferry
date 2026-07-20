"""兼容入口；资源定位由 infrastructure 所有。"""

from .infrastructure.resources import resource_path, resource_root

__all__ = ["resource_path", "resource_root"]
