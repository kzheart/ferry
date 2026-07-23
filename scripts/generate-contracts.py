#!/usr/bin/env python3
"""从 contracts/ 生成各运行时使用的静态契约常量。"""
from __future__ import annotations

import argparse
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
AGENTS_SOURCE = ROOT / "contracts" / "agents.json"
OUTPUTS = {
    ROOT / "app/src/api/contract/generated/agents.js": "frontend",
    ROOT / "app/src-tauri/src/contracts/agents.rs": "rust",
    ROOT / "engine/contracts/agents.py": "python",
    ROOT / "agent-runtime/src/contracts/agents.ts": "runtime",
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
    return agents


def frontend(agents: list[dict[str, object]]) -> str:
    payload = {
        agent["id"]: {
            "displayName": agent["display_name"],
            "icon": agent["icon"],
            "referenceKind": agent["reference_kind"],
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
        for key in ("display_name", "icon", "source_path", "reference_kind"):
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
        "",
    ))


def generated_contents(agents: list[dict[str, object]]) -> dict[Path, str]:
    return {
        path: {"frontend": frontend, "rust": rust, "python": python, "runtime": runtime}[kind](agents)
        for path, kind in OUTPUTS.items()
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    contents = generated_contents(load_agents())
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
