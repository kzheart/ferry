"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""
from __future__ import annotations

OPERATION_PLAN_ID_PREFIX = 'op_'
OPERATION_KINDS = frozenset(('edit', 'migration', 'metadata', 'delete', 'restore-delete'))
EDIT_OPERATION_KINDS = frozenset(('delete-turn', 'rewrite', 'replace-assistant-reply'))
OPERATION_STATUSES = frozenset(('planned', 'queued', 'applying', 'applied', 'failed', 'cancelled', 'expired'))
OPERATION_TERMINAL_STATUSES = frozenset(('applied', 'failed', 'cancelled', 'expired'))
OPERATION_SUCCESS_STATUS = 'applied'
