"""损耗目录：声明规则与 lose() 产出的 code 必须闭环。"""
import re
from pathlib import Path

import pytest

from engine.adapters import registry
from engine.sessions import loss

ROOT = Path(__file__).resolve().parents[1]
ADAPTERS = ROOT / "engine/adapters"


def test_conflicting_declaration_is_rejected():
    loss.declare({"test.loss_conflict": loss.DEGRADED})
    loss.declare({"test.loss_conflict": loss.DEGRADED})
    with pytest.raises(ValueError, match="声明冲突"):
        loss.declare({"test.loss_conflict": loss.DROPPED})
    loss._OUTCOMES.pop("test.loss_conflict", None)


def test_unknown_outcome_is_rejected():
    with pytest.raises(ValueError, match="未知损耗后果"):
        loss.declare({"test.loss_bad": "maybe"})


def test_every_produced_loss_code_is_declared_by_its_owner():
    """每个 lose() 的 code 都必须已声明后果，或明确属于不计入差异的信息事件。"""
    informational = {
        "migration.tool_degraded",      # 由 RenderDecision 单独统计
        "session.malformed_record",     # 原生文件坏行，读取期告警
        "session.orphan_tool_result",   # 读取期告警，不构成迁移差异
        "session.subagent_unlinked",
    }
    registry.create_registry()
    produced = set()
    for path in ADAPTERS.rglob("*.py"):
        for match in re.finditer(r'lose\(\s*"([a-z][a-z._]+)"', path.read_text(encoding="utf-8")):
            produced.add(match.group(1))
    undeclared = produced - set(loss._OUTCOMES) - informational
    assert not undeclared, f"未声明后果的 loss code: {sorted(undeclared)}"
