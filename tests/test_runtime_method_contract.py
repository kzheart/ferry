import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def _methods():
    document = json.loads((ROOT / "contracts/runtime-methods.json").read_text())
    return document["methods"]


def test_runtime_router_exactly_implements_contract_methods():
    expected = {method["name"] for method in _methods()}
    router = (ROOT / "ferry-runtime/src/runtime/command-router.ts").read_text()
    assert set(re.findall(r'case "([^"]+)":', router)) == expected


def test_internal_runtime_commands_never_enter_webview_allowlist():
    methods = _methods()
    public = {method["name"] for method in methods if method["exposure"] == "public"}
    internal = {method["name"] for method in methods if method["exposure"] == "internal"}
    assert internal == {"tool.result"}

    frontend = (
        ROOT / "app/src/shared/contracts/generated/runtime-methods.ts"
    ).read_text()
    for method in public:
        assert json.dumps(method) in frontend
    for method in internal:
        assert json.dumps(method) not in frontend

    host = (ROOT / "app/src-tauri/src/runtime/mod.rs").read_text()
    assert "runtime_methods::is_public(method)" in host
    assert 'method,\n        "health"' not in host


def test_runtime_parser_uses_generated_method_union():
    messages = (ROOT / "ferry-runtime/src/server/messages.ts").read_text()
    assert "type RuntimeMethod" in messages
    assert "isRuntimeMethod(input.method)" in messages
    assert "export type CommandMethod =" not in messages
    assert "const methods: readonly string[]" not in messages
