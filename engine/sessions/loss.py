"""会话保真度损耗目录：谁产生 loss code，谁声明它的后果。

共享迁移层不持有任何 Agent 私有 code。某个 Adapter 独有的降级/丢弃语义在
产生它的模块里 `declare()`，该模块被 import 是产生该 code 的前提，因此目录
不依赖装配顺序。重复声明必须一致，避免同一 code 在两处得到不同后果。
"""
from __future__ import annotations

DEGRADED = "degraded"
DROPPED = "dropped"

_OUTCOMES: dict[str, str] = {}


def declare(outcomes: dict[str, str]) -> None:
    for code, value in outcomes.items():
        if value not in (DEGRADED, DROPPED):
            raise ValueError(f"未知损耗后果: {code}={value!r}")
        existing = _OUTCOMES.get(code)
        if existing is not None and existing != value:
            raise ValueError(
                f"损耗后果声明冲突: {code} 已声明为 {existing!r}，"
                f"又声明为 {value!r}"
            )
        _OUTCOMES[code] = value


def outcome(loss) -> str | None:
    """返回一条损耗记录的保真度后果；未声明的 code 不计入迁移差异。"""
    code = loss.get("code") if isinstance(loss, dict) else None
    return _OUTCOMES.get(code)


# 跨 Adapter 通用的 canonical 损耗语义。
declare({
    "migration.children_not_migrated": DROPPED,
    "migration.fork_parent_fallback": DEGRADED,
    "migration.reasoning_dropped": DROPPED,
    "migration.reasoning_metadata_dropped": DEGRADED,
    "migration.truncated": DROPPED,
    "migration.unknown_block_dropped": DROPPED,
    "session.child_foreign_ignored": DROPPED,
    "session.child_parent_conflict": DROPPED,
    "session.unpaired_tool_use": DEGRADED,
})
