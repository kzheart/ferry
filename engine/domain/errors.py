"""跨应用与适配器边界共享的领域错误：code + params，供 RPC 结构化下发。"""
from __future__ import annotations


class DomainError(Exception):
    """所有领域异常的基类。

    code 是稳定的机器码（如 session.concurrent_modification），
    params 只放语义字段；message 仅用于日志与开发调试。
    """

    code = "internal.unexpected"
    category = "internal"
    retryable = False

    def __init__(self, message: str = "", params: dict | None = None):
        super().__init__(message or self.code)
        self.params = params or {}


class ConcurrentModificationError(DomainError, RuntimeError):
    """源会话在加载后发生变化；不得用旧快照覆盖新内容。"""

    code = "session.concurrent_modification"
    category = "conflict"
    retryable = True


class CapabilityUnsupportedError(DomainError, ValueError):
    """插件未提供请求的能力。"""

    code = "edit.operation_unsupported"
    category = "unsupported"

    def __init__(self, tool: str, capability: str):
        super().__init__(f"{tool} 不支持能力 {capability}",
                         {"tool": tool, "capability": capability})


class InvalidJsonError(DomainError, ValueError):
    code = "rpc.invalid_json"
    category = "validation"


class UnknownMethodError(DomainError, ValueError):
    code = "rpc.unknown_method"
    category = "validation"

    def __init__(self, method):
        super().__init__(f"未知 method: {method}", {"method": method})


class MissingParamError(DomainError, ValueError):
    code = "rpc.missing_param"
    category = "validation"

    def __init__(self, param: str):
        super().__init__(f"缺少参数: {param}", {"param": param})


class ToolUnknownError(DomainError, ValueError):
    code = "tool.unknown"
    category = "not-found"

    def __init__(self, tool):
        super().__init__(f"未知工具: {tool}", {"tool": tool})


class SessionNotFoundError(DomainError, ValueError):
    code = "session.not_found"
    category = "not-found"

    def __init__(self, tool: str, ref: str):
        super().__init__(f"找不到 {tool} 会话: {ref}",
                         {"tool": tool, "ref": ref})


class LocatorStaleError(DomainError, ValueError):
    """UI 持有的定位符与当前会话不再匹配。"""

    code = "session.locator_stale"
    category = "conflict"
    retryable = True

    def __init__(self, message="turn locator 已失效，请刷新会话",
                 params: dict | None = None):
        super().__init__(message, params)


class TurnOutOfRangeError(DomainError, ValueError):
    code = "edit.turn_out_of_range"
    category = "validation"

    def __init__(self, requested_turn, turn_count: int | None = None):
        params = {"requested_turn": requested_turn}
        if turn_count is not None:
            params["turn_count"] = turn_count
            message = f"轮次超界: 共 {turn_count} 轮"
        else:
            message = "turn 必须是正整数"
        super().__init__(message, params)


class OperationUnsupportedError(DomainError, ValueError):
    code = "edit.operation_unsupported"
    category = "unsupported"

    def __init__(self, tool: str, operation: str, mode: str | None = None):
        message = f"{tool} 不支持操作 {operation}" + (f"（{mode}）" if mode else "")
        super().__init__(message, {"tool": tool, "operation": operation,
                                   **({"mode": mode} if mode else {})})


class InvalidReplyError(DomainError, ValueError):
    code = "authoring.invalid_reply"
    category = "validation"


class SubagentNotSupportedError(DomainError, ValueError):
    code = "authoring.subagent_not_supported"
    category = "unsupported"


class SnapshotInvalidSourceError(DomainError, ValueError):
    code = "snapshot.invalid_source"
    category = "validation"
