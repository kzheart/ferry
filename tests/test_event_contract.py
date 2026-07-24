import json
from pathlib import Path

from engine.contracts.events import FERRY_EVENT_POLICIES, FERRY_EVENT_TYPES


ROOT = Path(__file__).resolve().parents[1]


def test_event_contract_is_generated_for_every_runtime():
    source = json.loads((ROOT / "contracts/events.json").read_text())
    event_types = [event["type"] for event in source["events"]]
    assert event_types == sorted(event_types)
    assert set(event_types) == FERRY_EVENT_TYPES
    assert {
        event["type"]: {
            "source": event["source"],
            "forward_to_ui": event["forward_to_ui"],
        }
        for event in source["events"]
    } == FERRY_EVENT_POLICIES

    for path in (
        "app/src/shared/contracts/generated/events.ts",
        "app/src-tauri/src/contracts/events.rs",
        "engine/contracts/events.py",
        "ferry-runtime/src/server/generated/events.ts",
    ):
        assert (ROOT / path).is_file()


def test_event_routing_uses_generated_policy():
    assert FERRY_EVENT_POLICIES["engine.request"] == {
        "source": "runtime",
        "forward_to_ui": False,
    }
    assert all(
        policy["forward_to_ui"]
        for policy in FERRY_EVENT_POLICIES.values()
        if policy["source"] == "host"
    )

    rust = "\n".join(
        path.read_text()
        for path in (ROOT / "app/src-tauri/src/runtime").glob("*.rs")
    )
    frontend = (ROOT / "app/src/platform/desktop/client.ts").read_text()
    runtime_messages = (
        ROOT / "ferry-runtime/src/server/messages.ts"
    ).read_text()
    assert "event_policy(event_type)" in rust
    assert "EventSource::Runtime" in rust
    assert "EventSource::Host" in rust
    assert "isFerryEventType" in frontend
    assert "type: RuntimeEventType" in runtime_messages
