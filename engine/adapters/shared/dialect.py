"""声明式工具方言：每个 adapter 一份数据，读端归一与写端渲染由同一份声明推导。

设计约定：
- 一个 OpBinding 描述"某个规范操作在该 adapter 里叫什么、字段怎么对应"。
  九成映射是纯字段改名/单位换算（FieldMap + 命名转换器）；装不进表的怪例
  （多字段派生、列表拆装）用 decode_hook/encode_hook 显式声明，不硬塞。
- parse 返回 None 表示"该调用无法无损归一"，调用方兜底成 TOOL_INVOKE 私有
  调用，原始参数全量保留——归一失败不等于信息丢失。
- render 返回 None 表示"目标端没有这个操作的原生形态"，由迁移层决定降级方式。
- 转换器是有限枚举的命名函数，未来用户自定义映射只允许引用这些名字，
  不允许注入任意代码。
"""
from __future__ import annotations

import importlib
import shlex
from dataclasses import dataclass
from typing import Any, Callable

_MISSING = object()
SKIP = object()


def _to_int(value):
    if isinstance(value, bool):
        return SKIP
    try:
        return int(float(value))
    except (TypeError, ValueError):
        return SKIP


def _numeric(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


CONVERTERS: dict[str, Callable[[Any], Any]] = {
    "int": _to_int,
    "str": lambda value: value if isinstance(value, str) else str(value),
    "s_to_ms": lambda value: int(value * 1000) if _numeric(value) else SKIP,
    "ms_to_s": lambda value: value / 1000 if _numeric(value) else SKIP,
    "sandbox_flag": lambda value: (
        "dangerously-disable" if value else "default"),
    "sandbox_unflag": lambda value: value == "dangerously-disable",
    "bool": lambda value: (
        value if isinstance(value, bool)
        else value.lower() == "true" if isinstance(value, str)
        and value.lower() in {"true", "false"} else SKIP),
}


def _convert(name: str | None, value):
    if name is None:
        return value
    return CONVERTERS[name](value)


@dataclass(frozen=True)
class FieldMap:
    """单个字段的双向对应。native 缺省时与 canonical 同名。"""

    canonical: str
    native: str | None = None
    decode: str | None = None
    encode: str | None = None
    read_alt: tuple[str, ...] = ()
    read_default: Any = _MISSING
    write_default: Any = _MISSING
    write: bool = True

    @property
    def native_name(self) -> str:
        return self.native if self.native is not None else self.canonical

    @property
    def read_names(self) -> tuple[str, ...]:
        return (self.native_name, *self.read_alt)


@dataclass(frozen=True)
class OpBinding:
    """一个规范操作在该 adapter 的原生形态。"""

    op: str
    name: str
    fields: tuple[FieldMap, ...] = ()
    read_names: tuple[str, ...] = ()
    extras: str = "ignore"  # ignore: 丢弃表外原生字段；fallback: 整体退回 TOOL_INVOKE
    readonly: bool = False  # 只用于读端归一（如旧代工具名），写端不再产出
    decode_hook: Callable[[dict], dict | None] | None = None
    encode_hook: Callable[[dict], dict | None] | None = None
    render_flags: Callable[[dict, dict], dict] | None = None
    # 字段映射完成后的原生入参后处理（如把 workdir 内联进命令）。
    # encode_post_fields 声明后处理额外消化的规范字段，计入 supported。
    encode_post: Callable[[dict, dict], dict] | None = None
    encode_post_fields: tuple[str, ...] = ()

    @property
    def all_read_names(self) -> tuple[str, ...]:
        return (self.name, *self.read_names)

    def parse(self, raw: dict) -> dict | None:
        if self.decode_hook is not None:
            return self.decode_hook(raw)
        canonical = {}
        consumed = set()
        for field in self.fields:
            key = next((name for name in field.read_names if name in raw),
                       None)
            if key is None:
                if field.read_default is not _MISSING:
                    canonical[field.canonical] = field.read_default
                continue
            consumed.add(key)
            value = _convert(field.decode, raw[key])
            if value is not SKIP:
                canonical[field.canonical] = value
        if self.extras == "fallback" and set(raw) - consumed:
            return None
        return canonical

    def render(self, canonical: dict) -> dict | None:
        if self.encode_hook is not None:
            return self.encode_hook(canonical)
        native = {}
        for field in self.fields:
            if not field.write:
                continue
            if field.canonical in canonical:
                value = _convert(field.encode, canonical[field.canonical])
                if value is not SKIP:
                    native[field.native_name] = value
            elif field.write_default is not _MISSING:
                native[field.native_name] = field.write_default
        if self.encode_post is not None:
            native = self.encode_post(canonical, native)
        return native

    def supported_fields(self) -> frozenset[str]:
        if self.encode_hook is not None or self.decode_hook is not None:
            return frozenset(field.canonical for field in self.fields)
        return frozenset(
            field.canonical for field in self.fields
            if field.write) | frozenset(self.encode_post_fields)


@dataclass(frozen=True)
class ToolDialect:
    """一个 adapter 的完整工具方言。"""

    adapter: str
    namespace: str
    bindings: tuple[OpBinding, ...]
    # 严格模式:入参不是 dict 时整体退回 TOOL_INVOKE(pi/grok);
    # 宽松模式:保留已识别的 op、原样透传入参,交由入参校验降级(claude/opencode)。
    strict_input: bool = False
    # 解析前丢弃的传输层键(如 grok updates 流的 variant 判别符):
    # 它们是记录格式的痕迹而非调用参数,不参与 extras 守卫。
    drop_native: tuple[str, ...] = ()

    def __post_init__(self):
        by_name = {}
        by_op = {}
        for binding in self.bindings:
            for name in binding.all_read_names:
                by_name[name] = binding
            if not binding.readonly:
                by_op.setdefault(binding.op, binding)
        object.__setattr__(self, "_by_read_name", by_name)
        object.__setattr__(self, "_by_op", by_op)

    def op_for(self, name: str) -> str | None:
        binding = self._by_read_name.get(name)
        return binding.op if binding else None

    def parse(self, name: str, raw) -> tuple[str, Any] | None:
        """原生调用 -> (规范操作, 规范入参)；None 表示应退回 TOOL_INVOKE。"""
        binding = self._by_read_name.get(name)
        if binding is None:
            return None
        if not isinstance(raw, dict):
            return None if self.strict_input else (binding.op, raw)
        # null 值即"未设置":有些格式(grok updates 流)会把完整参数
        # schema 连 null 一起写出,它们不携带信息,不该触发 extras 守卫。
        raw = {key: value for key, value in raw.items()
               if value is not None and key not in self.drop_native}
        canonical = binding.parse(raw)
        if canonical is None:
            return None
        return binding.op, canonical

    def render(self, op: str, canonical: dict) -> tuple[str, dict] | None:
        """规范调用 -> (原生工具名, 原生入参)；None 表示无原生形态。"""
        binding = self._by_op.get(op)
        if binding is None or not isinstance(canonical, dict):
            return None
        native = binding.render(canonical)
        if native is None:
            return None
        return binding.name, native

    def binding_for(self, op: str) -> OpBinding | None:
        return self._by_op.get(op)

    def write_ops(self) -> frozenset[str]:
        return frozenset(self._by_op)

    def supported_fields(self, op: str) -> frozenset[str]:
        binding = self._by_op.get(op)
        return binding.supported_fields() if binding else frozenset()


def inline_workdir(canonical: dict, native: dict,
                   command_key: str = "command") -> dict:
    """目标端 shell 没有工作目录参数时,把 workdir 前缀成 cd 保住语义。"""
    workdir = canonical.get("workdir")
    if not workdir:
        return native
    native = dict(native)
    native[command_key] = (
        f"cd {shlex.quote(str(workdir))} && {native.get(command_key, '')}")
    return native


def workdir_inline_flags(canonical: dict, _native: dict) -> dict:
    """workdir 被改写进命令时,把保真度如实标成 transformed。"""
    if not canonical.get("workdir"):
        return {}
    return {"_fidelity": "transformed",
            "_reason_codes": ("workdir_inlined",)}


def get_dialect(adapter: str) -> ToolDialect | None:
    """按 adapter 名懒加载其方言声明，避免 shared 反向依赖 adapter 包。"""
    try:
        module = importlib.import_module(
            f"engine.adapters.{adapter}.dialect")
    except ModuleNotFoundError:
        return None
    return module.DIALECT
