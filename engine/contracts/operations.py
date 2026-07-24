"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""
from __future__ import annotations

from typing import Literal, NotRequired, TypedDict

class TextReplyItem(TypedDict):
    kind: Literal['text']
    text: str

class ToolReplyItem(TypedDict):
    kind: Literal['tool']
    name: str
    input: dict[str, object] | str
    output: str

AssistantReplyItem = TextReplyItem | ToolReplyItem

class AssistantReply(TypedDict):
    items: list[AssistantReplyItem]

class MetadataPatch(TypedDict):
    name: NotRequired[str]
    pinned: NotRequired[bool]
    archived: NotRequired[bool]
    tags: NotRequired[list[str]]

class DeleteTurnOperation(TypedDict):
    op: Literal['delete-turn']
    turn: int

class RewriteOperation(TypedDict):
    op: Literal['rewrite']
    locator: str
    text: str

class ReplaceAssistantReplyOperation(TypedDict):
    op: Literal['replace-assistant-reply']
    turn: int | str
    reply: AssistantReply

EditOperation = DeleteTurnOperation | RewriteOperation | ReplaceAssistantReplyOperation

class EditOperationInput(TypedDict):
    kind: Literal['edit']
    tool: str
    ref: str
    ops: list[EditOperation]
    probe: NotRequired[bool]

class MigrationOperationInput(TypedDict):
    kind: Literal['migration']
    source_tool: str
    ref: str
    target_tool: str
    max_turn: NotRequired[int]
    probe: NotRequired[bool]
    probe_model: NotRequired[str]

class MetadataOperationInput(TypedDict):
    kind: Literal['metadata']
    tool: str
    ref: str
    patch: MetadataPatch

class DeleteOperationInput(TypedDict):
    kind: Literal['delete']
    tool: str
    ref: str

class RestoreDeleteOperationInput(TypedDict):
    kind: Literal['restore-delete']
    recovery_id: str

OperationInput = EditOperationInput | MigrationOperationInput | MetadataOperationInput | DeleteOperationInput | RestoreDeleteOperationInput

OPERATION_PLAN_ID_PREFIX = 'op_'
OPERATION_KINDS = frozenset(('edit', 'migration', 'metadata', 'delete', 'restore-delete'))
EDIT_OPERATION_KINDS = frozenset(('delete-turn', 'rewrite', 'replace-assistant-reply'))
OPERATION_STATUSES = frozenset(('planned', 'queued', 'applying', 'applied', 'failed', 'cancelled', 'expired'))
OPERATION_TERMINAL_STATUSES = frozenset(('applied', 'failed', 'cancelled', 'expired'))
OPERATION_SUCCESS_STATUS = 'applied'
