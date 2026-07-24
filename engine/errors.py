"""跨能力包共享的结构化错误：code + params，供 RPC 下发。"""
from __future__ import annotations

from .contracts.errors import error_policy


class DomainError(Exception):
    """所有领域异常的基类。

    code 是稳定的机器码（如 session.concurrent_modification），
    params 只放语义字段；message 仅用于日志与开发调试。
    """

    code = "internal.unexpected"

    def __init__(self, message: str = "", params: dict | None = None):
        super().__init__(message or self.code)
        self.params = params or {}
        policy = error_policy(self.code)
        self.category = policy["category"]
        self.retryable = policy["retryable"]


class ConcurrentModificationError(DomainError, RuntimeError):
    """源会话在加载后发生变化；不得用旧快照覆盖新内容。"""

    code = "session.concurrent_modification"


class InvalidJsonError(DomainError, ValueError):
    code = "rpc.invalid_json"


class InvalidRequestError(DomainError, ValueError):
    code = "rpc.invalid_request"


class UnsupportedProtocolError(DomainError, ValueError):
    code = "rpc.unsupported_protocol"

    def __init__(self, expected: str, actual):
        super().__init__(
            "IPC protocol 不匹配",
            {"expected": expected, "actual": actual},
        )


class UnknownMethodError(DomainError, ValueError):
    code = "rpc.unknown_method"

    def __init__(self, method):
        super().__init__(f"未知 method: {method}", {"method": method})


class MissingParamError(DomainError, ValueError):
    code = "rpc.missing_param"

    def __init__(self, param: str):
        super().__init__(f"缺少参数: {param}", {"param": param})


class ToolUnknownError(DomainError, ValueError):
    code = "tool.unknown"

    def __init__(self, tool):
        super().__init__(f"未知工具: {tool}", {"tool": tool})


class SessionNotFoundError(DomainError, ValueError):
    code = "session.not_found"

    def __init__(self, tool: str, ref: str):
        super().__init__(f"找不到 {tool} 会话: {ref}",
                         {"tool": tool, "ref": ref})


class SessionStoreUnavailableError(DomainError, RuntimeError):
    code = "session.store_unavailable"

    def __init__(self, tool: str, reason: str):
        super().__init__(
            f"{tool} 会话存储不可用: {reason}",
            {"tool": tool, "reason": reason},
        )


class AgentFormatChangedError(DomainError, RuntimeError):
    code = "agent.format_changed"

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

    def __init__(self, asset_id: str):
        super().__init__("找不到会话图片", {"asset_id": asset_id})


class LocatorStaleError(DomainError, ValueError):
    """UI 持有的定位符与当前会话不再匹配。"""

    code = "session.locator_stale"

    def __init__(self, message="turn locator 已失效，请刷新会话",
                 params: dict | None = None):
        super().__init__(message, params)


class TurnOutOfRangeError(DomainError, ValueError):
    code = "edit.turn_out_of_range"

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

    def __init__(self, tool: str, operation: str, mode: str | None = None):
        message = f"{tool} 不支持操作 {operation}" + (f"（{mode}）" if mode else "")
        super().__init__(message, {"tool": tool, "operation": operation,
                                   **({"mode": mode} if mode else {})})


class InvalidReplyError(DomainError, ValueError):
    code = "edit.invalid_reply"


class SubagentNotSupportedError(DomainError, ValueError):
    code = "edit.subagent_not_supported"


class SnapshotInvalidSourceError(DomainError, ValueError):
    code = "snapshot.invalid_source"


class AgentReferenceError(DomainError, ValueError):
    """Agent 只能使用当前 Engine 扫描索引签发的 opaque ref。"""

    code = "agent.reference_invalid"


class AgentRequestError(DomainError, ValueError):
    code = "agent.request_invalid"


class AgentApprovalError(DomainError, ValueError):
    code = "agent.approval_invalid"


class SummaryBackboneMissingError(DomainError, ValueError):
    """尚未为该会话建立摘要底座，无法写回蒸馏摘要。"""

    code = "summary.backbone_missing"


class OrganizationProposalError(DomainError, ValueError):
    """整理建议不完整、引用了过期摘要或提案状态不允许当前操作。"""

    code = "organization.proposal_invalid"


class OrganizationProposalNotFoundError(DomainError, ValueError):
    code = "organization.proposal_not_found"


class OrganizationProposalStaleError(DomainError, RuntimeError):
    code = "organization.proposal_stale"
