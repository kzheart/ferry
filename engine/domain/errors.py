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


class SessionStoreUnavailableError(DomainError, RuntimeError):
    code = "session.store_unavailable"
    category = "unavailable"
    retryable = True

    def __init__(self, tool: str, reason: str):
        super().__init__(
            f"{tool} 会话存储不可用: {reason}",
            {"tool": tool, "reason": reason},
        )


class AgentFormatChangedError(DomainError, RuntimeError):
    code = "agent.format_changed"
    category = "unsupported"

    def __init__(self, agent: str, location: str, expected, actual):
        super().__init__(
            f"{agent} 当前结构不匹配: {location}",
            {
                "agent": agent,
                "location": location,
                "expected": expected,
                "actual": actual,
            },
        )


class SessionAssetNotFoundError(DomainError, ValueError):
    code = "session.asset_not_found"
    category = "not-found"

    def __init__(self, asset_id: str):
        super().__init__("找不到会话图片", {"asset_id": asset_id})


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
    code = "edit.invalid_reply"
    category = "validation"


class SubagentNotSupportedError(DomainError, ValueError):
    code = "edit.subagent_not_supported"
    category = "unsupported"


class SnapshotInvalidSourceError(DomainError, ValueError):
    code = "snapshot.invalid_source"
    category = "validation"


class AgentReferenceError(DomainError, ValueError):
    """Agent 只能使用当前 Engine 扫描索引签发的 opaque ref。"""

    code = "agent.reference_invalid"
    category = "validation"


class AgentRequestError(DomainError, ValueError):
    code = "agent.request_invalid"
    category = "validation"


class AgentApprovalError(DomainError, ValueError):
    code = "agent.approval_invalid"
    category = "permission"


class SummaryBackboneMissingError(DomainError, ValueError):
    """尚未为该会话建立摘要底座，无法写回蒸馏摘要。"""

    code = "summary.backbone_missing"
    category = "not-found"


class OrganizationProposalError(DomainError, ValueError):
    """整理建议不完整、引用了过期摘要或提案状态不允许当前操作。"""

    code = "organization.proposal_invalid"
    category = "validation"


class OrganizationProposalNotFoundError(DomainError, ValueError):
    code = "organization.proposal_not_found"
    category = "not-found"


class OrganizationProposalStaleError(DomainError, RuntimeError):
    code = "organization.proposal_stale"
    category = "conflict"
    retryable = True
