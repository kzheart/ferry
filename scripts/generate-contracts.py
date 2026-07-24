#!/usr/bin/env python3
"""从 contracts/ 生成各运行时使用的静态契约常量。"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
AGENTS_SOURCE = ROOT / "contracts" / "agents.json"
ENGINE_METHODS_SOURCE = ROOT / "contracts" / "engine-methods.json"
RUNTIME_METHODS_SOURCE = ROOT / "contracts" / "runtime-methods.json"
IPC_SOURCE = ROOT / "contracts" / "ipc.json"
SESSION_REF_SOURCE = ROOT / "contracts" / "session-ref.json"
OPERATIONS_SOURCE = ROOT / "contracts" / "operations.json"
AGENT_OUTPUTS = {
    ROOT / "app/src/api/contract/generated/agents.js": "frontend",
    ROOT / "app/src-tauri/src/contracts/agents.rs": "rust",
    ROOT / "engine/contracts/agents.py": "python",
    ROOT / "ferry-runtime/src/server/generated/agents.ts": "runtime",
}
ENGINE_METHOD_OUTPUTS = {
    ROOT / "app/src-tauri/src/contracts/engine_methods.rs": "rust",
    ROOT / "engine/contracts/engine_methods.py": "python",
}
RUNTIME_METHOD_OUTPUTS = {
    ROOT / "app/src/api/contract/generated/runtime-methods.js": "frontend",
    ROOT / "app/src-tauri/src/contracts/runtime_methods.rs": "rust",
    ROOT / "ferry-runtime/src/server/generated/runtime-methods.ts": "runtime",
}
IPC_OUTPUTS = {
    ROOT / "app/src/api/contract/generated/ipc.js": "frontend",
    ROOT / "app/src-tauri/src/contracts/ipc.rs": "rust",
    ROOT / "engine/contracts/ipc.py": "python",
    ROOT / "ferry-runtime/src/server/generated/ipc.ts": "runtime",
}
SESSION_REF_OUTPUTS = {
    ROOT / "app/src/api/contract/generated/session-ref.js": "frontend",
    ROOT / "app/src-tauri/src/contracts/session_ref.rs": "rust",
    ROOT / "engine/contracts/session_ref.py": "python",
    ROOT / "ferry-runtime/src/server/generated/session-ref.ts": "runtime",
}
OPERATIONS_OUTPUTS = {
    ROOT / "app/src/api/contract/generated/operations.js": "frontend",
    ROOT / "app/src-tauri/src/contracts/operations.rs": "rust",
    ROOT / "engine/contracts/operations.py": "python",
    ROOT / "ferry-runtime/src/server/generated/operations.ts": "runtime",
}


def load_agents() -> list[dict[str, object]]:
    document = json.loads(AGENTS_SOURCE.read_text())
    agents = document.get("agents")
    if not isinstance(agents, list) or not agents:
        raise ValueError("contracts/agents.json 必须包含非空 agents 数组")
    identifiers = [agent.get("id") for agent in agents]
    if len(identifiers) != len(set(identifiers)) or not all(
        isinstance(identifier, str) and identifier for identifier in identifiers
    ):
        raise ValueError("Agent id 必须唯一且非空")
    required = {
        "id", "display_name", "icon", "source_path", "executables",
        "fallback_bin_dirs",
    }
    if any(not isinstance(agent, dict) or set(agent) != required for agent in agents):
        raise ValueError("Agent 契约字段必须精确为当前静态定义")
    return agents


def load_engine_methods() -> list[dict[str, object]]:
    document = json.loads(ENGINE_METHODS_SOURCE.read_text())
    methods = document.get("methods")
    if not isinstance(methods, list) or not methods:
        raise ValueError("contracts/engine-methods.json 必须包含非空 methods 数组")
    required = {"name", "kind", "exposure", "timeout", "retry", "dispatch"}
    allowed_kinds = {"read", "index-refresh", "mutation"}
    allowed_exposures = {"public", "trusted-ui", "internal"}
    allowed_timeouts = {"normal", "lookup"}
    allowed_retries = {"safe-read", "never"}
    allowed_dispatches = {"parallel-read", "serial"}
    names: list[str] = []
    for method in methods:
        if not isinstance(method, dict) or set(method) != required:
            raise ValueError(
                "Engine 方法契约字段必须精确为 "
                "name/kind/exposure/timeout/retry/dispatch"
            )
        name = method["name"]
        if not isinstance(name, str) or not name:
            raise ValueError("Engine method name 必须非空")
        if method["kind"] not in allowed_kinds:
            raise ValueError(f"Engine method {name} 的 kind 无效")
        if method["exposure"] not in allowed_exposures:
            raise ValueError(f"Engine method {name} 的 exposure 无效")
        if method["timeout"] not in allowed_timeouts:
            raise ValueError(f"Engine method {name} 的 timeout 无效")
        if method["retry"] not in allowed_retries:
            raise ValueError(f"Engine method {name} 的 retry 无效")
        if method["dispatch"] not in allowed_dispatches:
            raise ValueError(f"Engine method {name} 的 dispatch 无效")
        if method["dispatch"] == "parallel-read" and method["kind"] != "read":
            raise ValueError(f"Engine method {name} 的 parallel-read 只能用于 read")
        names.append(name)
    if len(names) != len(set(names)):
        raise ValueError("Engine method name 必须唯一")
    return methods


def load_runtime_methods() -> list[dict[str, object]]:
    document = json.loads(RUNTIME_METHODS_SOURCE.read_text())
    methods = document.get("methods")
    if not isinstance(methods, list) or not methods:
        raise ValueError("contracts/runtime-methods.json 必须包含非空 methods 数组")
    names: list[str] = []
    for method in methods:
        if not isinstance(method, dict) or set(method) != {"name", "exposure"}:
            raise ValueError("Runtime 方法契约字段必须精确为 name/exposure")
        name = method["name"]
        if not isinstance(name, str) or not name:
            raise ValueError("Runtime method name 必须非空")
        if method["exposure"] not in {"public", "internal"}:
            raise ValueError(f"Runtime method {name} 的 exposure 无效")
        names.append(name)
    if len(names) != len(set(names)):
        raise ValueError("Runtime method name 必须唯一")
    if not any(method["exposure"] == "internal" for method in methods):
        raise ValueError("Runtime 必须保留至少一个内部命令")
    return methods


def load_ipc() -> dict[str, object]:
    document = json.loads(IPC_SOURCE.read_text())
    expected = {
        "protocol": "ferry-ipc/1",
        "request": {
            "required": ["protocol", "id", "method", "params"],
            "additional_properties": False,
        },
        "response": {
            "success_required": ["protocol", "id", "ok", "result"],
            "failure_required": ["protocol", "id", "ok", "error"],
            "additional_properties": False,
        },
        "error": {
            "required": ["code", "category", "retryable", "params"],
            "additional_properties": False,
        },
        "event": {
            "required": ["protocol", "type", "payload"],
            "optional": ["correlation_id", "context"],
            "additional_properties": False,
        },
    }
    if document != expected:
        raise ValueError("IPC 契约字段必须精确为当前 envelope 定义")
    return document


def load_session_ref() -> dict[str, object]:
    document = json.loads(SESSION_REF_SOURCE.read_text())
    expected = {
        "opaque_prefix": "fsr_",
        "minimum_length": 8,
        "maximum_length": 128,
        "allowed_suffix": "ascii-alphanumeric-underscore-hyphen",
    }
    if document != expected:
        raise ValueError("SessionRef 契约字段必须精确为当前 opaque ref 定义")
    return document


def load_operations() -> dict[str, object]:
    document = json.loads(OPERATIONS_SOURCE.read_text())
    expected_keys = {
        "plan_id_prefix",
        "kinds",
        "edit_operations",
        "statuses",
        "terminal_statuses",
        "success_status",
    }
    if not isinstance(document, dict) or set(document) != expected_keys:
        raise ValueError("Operation 契约字段不完整")
    list_fields = ("kinds", "edit_operations", "statuses", "terminal_statuses")
    for field in list_fields:
        values = document[field]
        if (
            not isinstance(values, list)
            or not values
            or not all(isinstance(value, str) and value for value in values)
            or len(values) != len(set(values))
        ):
            raise ValueError(f"Operation {field} 必须是非空唯一字符串数组")
    if document["plan_id_prefix"] != "op_":
        raise ValueError("Operation plan_id_prefix 必须为 op_")
    statuses = set(document["statuses"])
    terminal = set(document["terminal_statuses"])
    if not terminal < statuses:
        raise ValueError("Operation terminal_statuses 必须是 statuses 的真子集")
    if document["success_status"] not in terminal:
        raise ValueError("Operation success_status 必须是终态")
    return document


def frontend(agents: list[dict[str, object]]) -> str:
    payload = {
        agent["id"]: {
            "displayName": agent["display_name"],
            "icon": agent["icon"],
        }
        for agent in agents
    }
    source = json.dumps(payload, ensure_ascii=False, indent=2)
    executables = [executable for agent in agents for executable in agent["executables"]]
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"export const AGENTS = Object.freeze({source});",
        "export const AGENT_IDS = Object.freeze(Object.keys(AGENTS));",
        f"export const ALLOWED_EXECUTABLES = Object.freeze({json.dumps(executables)});",
        "",
    ))


def rust(agents: list[dict[str, object]]) -> str:
    ids = ", ".join(f'\"{agent["id"]}\"' for agent in agents)
    executables = [executable for agent in agents for executable in agent["executables"]]
    allowed = ", ".join(f'\"{executable}\"' for executable in executables)
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"pub(crate) const AGENT_IDS: &[&str] = &[{ids}];",
        f"pub(crate) const ALLOWED_EXECUTABLES: &[&str] = &[{allowed}];",
        "",
    ))


def python(agents: list[dict[str, object]]) -> str:
    lines = [
        '"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""',
        "from __future__ import annotations",
        "",
        "AGENTS = {",
    ]
    for agent in agents:
        lines.append(f'    {agent["id"]!r}: {{')
        for key in ("display_name", "icon", "source_path"):
            lines.append(f"        {key!r}: {agent[key]!r},")
        lines.append(f"        'executables': {tuple(agent['executables'])!r},")
        lines.append(f"        'fallback_bin_dirs': {tuple(agent['fallback_bin_dirs'])!r},")
        lines.append("    },")
    lines.extend(("}", "AGENT_IDS = tuple(AGENTS)", ""))
    return "\n".join(lines)


def runtime(agents: list[dict[str, object]]) -> str:
    identifiers = [agent["id"] for agent in agents]
    labels = [agent["display_name"] for agent in agents]
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"export const AGENT_IDS = {json.dumps(identifiers)} as const;",
        f"export const AGENT_LABELS = {json.dumps(labels)} as const;",
        "export type AgentId = (typeof AGENT_IDS)[number];",
        "",
    ))


def engine_methods_rust(methods: list[dict[str, object]]) -> str:
    timeout_variants = {
        "normal": "Normal",
        "lookup": "Lookup",
    }
    used_timeout_variants = [
        timeout_variants[timeout]
        for timeout in timeout_variants
        if any(method["timeout"] == timeout for method in methods)
    ]
    rows = []
    for method in methods:
        exposure = {
            "public": "Public",
            "trusted-ui": "TrustedUi",
            "internal": "Internal",
        }[method["exposure"]]
        timeout = timeout_variants[method["timeout"]]
        retry = {"safe-read": "SafeRead", "never": "Never"}[method["retry"]]
        rows.extend((
            f'        {json.dumps(method["name"])} => Some(EngineMethodPolicy {{',
            f"            exposure: Exposure::{exposure},",
            f"            timeout: TimeoutClass::{timeout},",
            f"            retry: RetryPolicy::{retry},",
            "        }),",
        ))
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]",
        "pub(crate) enum Exposure {",
        "    Public,",
        "    TrustedUi,",
        "    Internal,",
        "}",
        "",
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]",
        "pub(crate) enum TimeoutClass {",
        *(f"    {variant}," for variant in used_timeout_variants),
        "}",
        "",
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]",
        "pub(crate) enum RetryPolicy {",
        "    SafeRead,",
        "    Never,",
        "}",
        "",
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]",
        "pub(crate) struct EngineMethodPolicy {",
        "    pub(crate) exposure: Exposure,",
        "    pub(crate) timeout: TimeoutClass,",
        "    pub(crate) retry: RetryPolicy,",
        "}",
        "",
        "pub(crate) fn policy(method: &str) -> Option<EngineMethodPolicy> {",
        "    match method {",
        *rows,
        "        _ => None,",
        "    }",
        "}",
        "",
    ))


def engine_methods_python(methods: list[dict[str, object]]) -> str:
    policies = {
        method["name"]: {
            key: method[key]
            for key in ("kind", "exposure", "timeout", "retry", "dispatch")
        }
        for method in methods
    }
    parallel = tuple(
        method["name"] for method in methods if method["dispatch"] == "parallel-read"
    )
    return "\n".join((
        '"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""',
        "from __future__ import annotations",
        "",
        f"ENGINE_METHOD_POLICIES = {policies!r}",
        "ENGINE_METHOD_NAMES = frozenset(ENGINE_METHOD_POLICIES)",
        f"PARALLEL_READ_METHOD_NAMES = frozenset({parallel!r})",
        "",
    ))


def runtime_methods_frontend(methods: list[dict[str, object]]) -> str:
    public = [method["name"] for method in methods if method["exposure"] == "public"]
    values = "[\n" + "\n".join(
        f"  {json.dumps(method)}," for method in public
    ) + "\n]"
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"export const PUBLIC_RUNTIME_METHODS = Object.freeze({values});",
        "export const isPublicRuntimeMethod = method =>",
        "  PUBLIC_RUNTIME_METHODS.includes(method);",
        "",
    ))


def runtime_methods_rust(methods: list[dict[str, object]]) -> str:
    public = [method["name"] for method in methods if method["exposure"] == "public"]
    values = "\n".join(f"    {json.dumps(method)}," for method in public)
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        "const PUBLIC_RUNTIME_METHODS: &[&str] = &[",
        values,
        "];",
        "",
        "pub(crate) fn is_public(method: &str) -> bool {",
        "    PUBLIC_RUNTIME_METHODS.contains(&method)",
        "}",
        "",
    ))


def runtime_methods_runtime(methods: list[dict[str, object]]) -> str:
    names = [method["name"] for method in methods]
    public = [method["name"] for method in methods if method["exposure"] == "public"]
    names_source = "[\n" + "\n".join(
        f"  {json.dumps(method)}," for method in names
    ) + "\n]"
    public_source = "[\n" + "\n".join(
        f"  {json.dumps(method)}," for method in public
    ) + "\n]"
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"export const RUNTIME_METHODS = {names_source} as const;",
        f"export const PUBLIC_RUNTIME_METHODS = {public_source} as const;",
        "export type RuntimeMethod = (typeof RUNTIME_METHODS)[number];",
        "export function isRuntimeMethod(method: unknown): method is RuntimeMethod {",
        "  return (",
        '    typeof method === "string" &&',
        "    (RUNTIME_METHODS as readonly string[]).includes(method)",
        "  );",
        "}",
        "",
    ))


def session_ref_frontend(contract: dict[str, object]) -> str:
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"export const OPAQUE_SESSION_REF_PREFIX = {json.dumps(contract['opaque_prefix'])};",
        f"export const OPAQUE_SESSION_REF_MIN_LENGTH = {contract['minimum_length']};",
        f"export const OPAQUE_SESSION_REF_MAX_LENGTH = {contract['maximum_length']};",
        "export const isOpaqueSessionRef = value =>",
        '  typeof value === "string" &&',
        "  value.length >= OPAQUE_SESSION_REF_MIN_LENGTH &&",
        "  value.length <= OPAQUE_SESSION_REF_MAX_LENGTH &&",
        "  value.startsWith(OPAQUE_SESSION_REF_PREFIX) &&",
        "  /^[A-Za-z0-9_-]+$/.test(value);",
        "",
    ))


def session_ref_rust(contract: dict[str, object]) -> str:
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f'const OPAQUE_SESSION_REF_PREFIX: &str = "{contract["opaque_prefix"]}";',
        f"const OPAQUE_SESSION_REF_MIN_LENGTH: usize = {contract['minimum_length']};",
        f"const OPAQUE_SESSION_REF_MAX_LENGTH: usize = {contract['maximum_length']};",
        "",
        "pub(crate) fn is_opaque_session_ref(value: &str) -> bool {",
        "    (OPAQUE_SESSION_REF_MIN_LENGTH..=OPAQUE_SESSION_REF_MAX_LENGTH).contains(&value.len())",
        "        && value.starts_with(OPAQUE_SESSION_REF_PREFIX)",
        "        && value",
        "            .bytes()",
        "            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))",
        "}",
        "",
    ))


def session_ref_python(contract: dict[str, object]) -> str:
    return "\n".join((
        '"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""',
        "from __future__ import annotations",
        "",
        f"OPAQUE_SESSION_REF_PREFIX = {contract['opaque_prefix']!r}",
        f"OPAQUE_SESSION_REF_MIN_LENGTH = {contract['minimum_length']}",
        f"OPAQUE_SESSION_REF_MAX_LENGTH = {contract['maximum_length']}",
        "",
        "def is_opaque_session_ref(value: object) -> bool:",
        "    return (",
        "        isinstance(value, str)",
        "        and OPAQUE_SESSION_REF_MIN_LENGTH <= len(value) <= OPAQUE_SESSION_REF_MAX_LENGTH",
        "        and value.startswith(OPAQUE_SESSION_REF_PREFIX)",
        "        and all(character.isascii() and (character.isalnum() or character in '_-')",
        "                for character in value)",
        "    )",
        "",
    ))


def session_ref_runtime(contract: dict[str, object]) -> str:
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        'import type { AgentId } from "./agents.js";',
        "",
        f'export const OPAQUE_SESSION_REF_PREFIX = "{contract["opaque_prefix"]}" as const;',
        f"export const OPAQUE_SESSION_REF_MIN_LENGTH = {contract['minimum_length']} as const;",
        f"export const OPAQUE_SESSION_REF_MAX_LENGTH = {contract['maximum_length']} as const;",
        "",
        "export interface SessionRef {",
        "  tool: AgentId;",
        "  ref: string;",
        "}",
        "",
        "export function isOpaqueSessionRef(value: unknown): value is string {",
        "  return (",
        '    typeof value === "string" &&',
        "    value.length >= OPAQUE_SESSION_REF_MIN_LENGTH &&",
        "    value.length <= OPAQUE_SESSION_REF_MAX_LENGTH &&",
        "    value.startsWith(OPAQUE_SESSION_REF_PREFIX) &&",
        "    /^[A-Za-z0-9_-]+$/.test(value)",
        "  );",
        "}",
        "",
    ))


def operations_frontend(contract: dict[str, object]) -> str:
    def array(values: list[str]) -> str:
        return "[\n" + "\n".join(
            f"  {json.dumps(value)}," for value in values
        ) + "\n]"

    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"export const OPERATION_PLAN_ID_PREFIX = {json.dumps(contract['plan_id_prefix'])};",
        f"export const OPERATION_KINDS = Object.freeze({array(contract['kinds'])});",
        "export const EDIT_OPERATION_KINDS = "
        f"Object.freeze({array(contract['edit_operations'])});",
        f"export const OPERATION_STATUSES = Object.freeze({array(contract['statuses'])});",
        "export const OPERATION_TERMINAL_STATUSES = "
        f"Object.freeze({array(contract['terminal_statuses'])});",
        f"export const OPERATION_SUCCESS_STATUS = {json.dumps(contract['success_status'])};",
        "",
    ))


def operations_rust(contract: dict[str, object]) -> str:
    def compact_declaration(name: str, values: list[str]) -> str:
        items = ", ".join(json.dumps(value) for value in values)
        return f"pub(crate) const {name}: &[&str] =\n    &[{items}];"

    def expanded_declaration(name: str, values: list[str]) -> str:
        body = "\n".join(f"    {json.dumps(value)}," for value in values)
        return f"pub(crate) const {name}: &[&str] = &[\n{body}\n];"

    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f'pub(crate) const OPERATION_PLAN_ID_PREFIX: &str = "{contract["plan_id_prefix"]}";',
        compact_declaration("OPERATION_KINDS", contract["kinds"]),
        compact_declaration("EDIT_OPERATION_KINDS", contract["edit_operations"]),
        expanded_declaration("OPERATION_STATUSES", contract["statuses"]),
        compact_declaration(
            "OPERATION_TERMINAL_STATUSES", contract["terminal_statuses"],
        ),
        f'pub(crate) const OPERATION_SUCCESS_STATUS: &str = "{contract["success_status"]}";',
        "",
    ))


def operations_python(contract: dict[str, object]) -> str:
    return "\n".join((
        '"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""',
        "from __future__ import annotations",
        "",
        f"OPERATION_PLAN_ID_PREFIX = {contract['plan_id_prefix']!r}",
        f"OPERATION_KINDS = frozenset({tuple(contract['kinds'])!r})",
        f"EDIT_OPERATION_KINDS = frozenset({tuple(contract['edit_operations'])!r})",
        f"OPERATION_STATUSES = frozenset({tuple(contract['statuses'])!r})",
        "OPERATION_TERMINAL_STATUSES = "
        f"frozenset({tuple(contract['terminal_statuses'])!r})",
        f"OPERATION_SUCCESS_STATUS = {contract['success_status']!r}",
        "",
    ))


def operations_runtime(contract: dict[str, object]) -> str:
    def array(values: list[str]) -> str:
        return "[\n" + "\n".join(
            f"  {json.dumps(value)}," for value in values
        ) + "\n]"

    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f'export const OPERATION_PLAN_ID_PREFIX = "{contract["plan_id_prefix"]}" as const;',
        f"export const OPERATION_KINDS = {array(contract['kinds'])} as const;",
        "export const EDIT_OPERATION_KINDS = "
        f"{array(contract['edit_operations'])} as const;",
        f"export const OPERATION_STATUSES = {array(contract['statuses'])} as const;",
        "export const OPERATION_TERMINAL_STATUSES = "
        f"{array(contract['terminal_statuses'])} as const;",
        f'export const OPERATION_SUCCESS_STATUS = "{contract["success_status"]}" as const;',
        "export type OperationKind = (typeof OPERATION_KINDS)[number];",
        "export type EditOperationKind = (typeof EDIT_OPERATION_KINDS)[number];",
        "export type OperationStatus = (typeof OPERATION_STATUSES)[number];",
        "",
    ))


def contract_hash(
    agents: list[dict[str, object]],
    methods: list[dict[str, object]],
    runtime_methods: list[dict[str, object]],
    ipc: dict[str, object],
    session_ref: dict[str, object],
    operations: dict[str, object],
) -> str:
    payload = json.dumps(
        {
            "agents": agents,
            "engine_methods": methods,
            "runtime_methods": runtime_methods,
            "ipc": ipc,
            "session_ref": session_ref,
            "operations": operations,
        },
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode()
    return "sha256:" + hashlib.sha256(payload).hexdigest()


def ipc_frontend(contract: dict[str, object], digest: str) -> str:
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f"export const FERRY_IPC_PROTOCOL = {json.dumps(contract['protocol'])};",
        f"export const FERRY_CONTRACT_HASH = {json.dumps(digest)};",
        "",
    ))


def ipc_rust(contract: dict[str, object], digest: str) -> str:
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f'pub(crate) const FERRY_IPC_PROTOCOL: &str = "{contract["protocol"]}";',
        "pub(crate) const FERRY_CONTRACT_HASH: &str =",
        f'    "{digest}";',
        "",
    ))


def ipc_python(contract: dict[str, object], digest: str) -> str:
    return "\n".join((
        '"""此文件由 scripts/generate-contracts.py 生成，请勿手改。"""',
        "from __future__ import annotations",
        "",
        f"FERRY_IPC_PROTOCOL = {contract['protocol']!r}",
        f"FERRY_CONTRACT_HASH = {digest!r}",
        "",
    ))


def ipc_runtime(contract: dict[str, object], digest: str) -> str:
    return "\n".join((
        "// 此文件由 scripts/generate-contracts.py 生成，请勿手改。",
        f'export const FERRY_IPC_PROTOCOL = "{contract["protocol"]}" as const;',
        "export const FERRY_CONTRACT_HASH =",
        f'  "{digest}" as const;',
        "",
        "export interface IpcRequest<Method extends string = string> {",
        "  protocol: typeof FERRY_IPC_PROTOCOL;",
        "  id: string;",
        "  method: Method;",
        "  params: Record<string, unknown>;",
        "}",
        "",
        "export interface IpcError {",
        "  code: string;",
        "  category: string;",
        "  retryable: boolean;",
        "  params: Record<string, unknown>;",
        "}",
        "",
        "export interface IpcSuccessResponse {",
        "  protocol: typeof FERRY_IPC_PROTOCOL;",
        "  id: string;",
        "  ok: true;",
        "  result: unknown;",
        "}",
        "",
        "export interface IpcFailureResponse {",
        "  protocol: typeof FERRY_IPC_PROTOCOL;",
        "  id: string;",
        "  ok: false;",
        "  error: IpcError;",
        "}",
        "",
        "export type IpcResponse = IpcSuccessResponse | IpcFailureResponse;",
        "",
        "export interface IpcEvent {",
        "  protocol: typeof FERRY_IPC_PROTOCOL;",
        "  type: string;",
        "  correlation_id?: string;",
        "  context?: Record<string, unknown>;",
        "  payload: Record<string, unknown>;",
        "}",
        "",
    ))


def generated_contents(
    agents: list[dict[str, object]],
    engine_methods: list[dict[str, object]],
    runtime_methods: list[dict[str, object]],
    ipc: dict[str, object],
    session_ref: dict[str, object],
    operations: dict[str, object],
) -> dict[Path, str]:
    agent_contents = {
        path: {"frontend": frontend, "rust": rust, "python": python, "runtime": runtime}[kind](agents)
        for path, kind in AGENT_OUTPUTS.items()
    }
    engine_contents = {
        path: {"rust": engine_methods_rust, "python": engine_methods_python}[kind](engine_methods)
        for path, kind in ENGINE_METHOD_OUTPUTS.items()
    }
    runtime_method_contents = {
        path: {
            "frontend": runtime_methods_frontend,
            "rust": runtime_methods_rust,
            "runtime": runtime_methods_runtime,
        }[kind](runtime_methods)
        for path, kind in RUNTIME_METHOD_OUTPUTS.items()
    }
    session_ref_contents = {
        path: {
            "frontend": session_ref_frontend,
            "rust": session_ref_rust,
            "python": session_ref_python,
            "runtime": session_ref_runtime,
        }[kind](session_ref)
        for path, kind in SESSION_REF_OUTPUTS.items()
    }
    operation_contents = {
        path: {
            "frontend": operations_frontend,
            "rust": operations_rust,
            "python": operations_python,
            "runtime": operations_runtime,
        }[kind](operations)
        for path, kind in OPERATIONS_OUTPUTS.items()
    }
    digest = contract_hash(
        agents, engine_methods, runtime_methods, ipc, session_ref, operations,
    )
    ipc_contents = {
        path: {
            "frontend": ipc_frontend,
            "rust": ipc_rust,
            "python": ipc_python,
            "runtime": ipc_runtime,
        }[kind](ipc, digest)
        for path, kind in IPC_OUTPUTS.items()
    }
    return (
        agent_contents
        | engine_contents
        | runtime_method_contents
        | session_ref_contents
        | operation_contents
        | ipc_contents
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    contents = generated_contents(
        load_agents(),
        load_engine_methods(),
        load_runtime_methods(),
        load_ipc(),
        load_session_ref(),
        load_operations(),
    )
    stale = [path for path, content in contents.items() if not path.exists() or path.read_text() != content]
    if args.check:
        if stale:
            relative = ", ".join(str(path.relative_to(ROOT)) for path in stale)
            raise SystemExit(f"生成契约已漂移: {relative}")
        return 0
    for path, content in contents.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
