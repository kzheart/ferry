"""由小端口组合而成的工具适配器注册项。"""

from dataclasses import dataclass
from typing import Callable, Protocol


class Editor(Protocol):
    name: str


@dataclass
class ToolAdapter:
    identity: str
    source_path: str
    scanner: Callable[[object], list[dict]]
    reader: Callable
    writer: Callable
    editor: Editor
    verifier: Callable
    model_provider: Callable
    fallback_models: Callable[[], list[dict]]
    resolve_ref: Callable[[str], str]
    resume_descriptor: Callable[[str, str], dict]
    cleanup: Callable[[str, object], None]
    probe_edited: Callable
    validation_ref: Callable[[str, object], str]
