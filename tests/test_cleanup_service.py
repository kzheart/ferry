from types import SimpleNamespace

import pytest

from engine.errors import AgentReferenceError, AgentRequestError
from engine.sessions.cleanup import CleanupService
from engine.sessions.index import IndexedSession


class FakeIndex:
    def __init__(self, records):
        self.records = records
        self.generation = 0

    def snapshot_with_status(self):
        return ({"claude": {"ok": True}, "opencode": {"ok": True}}, self.records, self.generation)

    def refresh(self):
        return self.records

    def resolve(self, tool, ref, *, pin_content=True):
        for record in self.records:
            if record.tool == tool and record.opaque_ref == ref:
                return record
        raise AgentReferenceError("unknown ref")


def _record(tool, session_id, ref, updated, project, title):
    return IndexedSession(
        opaque_ref=ref,
        tool=tool,
        canonical_ref=f"/sessions/{tool}/{session_id}",
        root=None,
        storage_kind="memory",
        row={
            "id": session_id,
            "title": title,
            "dir": project,
            "updated": updated,
            "created": updated - 100,
            "count": 4,
            "size": 100,
        },
        revision=f"revision-{session_id}",
        source_identity=None,
    )


@pytest.fixture
def cleanup_environment():
    records = [
        _record("claude", "c1", "fsr_c100", 400, "/project-a", "C1"),
        _record("opencode", "o1", "fsr_o100", 300, "/project-a", "O1"),
        _record("claude", "c2", "fsr_c200", 200, "/project-b", "C2"),
        _record("opencode", "o2", "fsr_o200", 100, "/project-a", "O2"),
    ]
    index = FakeIndex(records)
    ports = SimpleNamespace(adapters=lambda: ("claude", "opencode"))
    return index, CleanupService(index, ports)


def test_inventory_paginates_stably_and_union_equals_total(cleanup_environment):
    _index, service = cleanup_environment

    first = service.inventory(None, page_size=2)
    second = service.inventory(None, cursor=first["next_cursor"], page_size=2)
    repeated = service.inventory(None, page_size=2)

    assert first["scope_id"] == second["scope_id"] == repeated["scope_id"]
    assert first["total"] == 4
    assert first["covered"] == 0
    assert [row["id"] for row in first["page"]] == ["c1", "o1"]
    assert [row["id"] for row in second["page"]] == ["c2", "o2"]
    assert {row["id"] for row in first["page"] + second["page"]} == {
        "c1", "o1", "c2", "o2",
    }
    assert second["next_cursor"] is None
    assert repeated["page"] == first["page"]


def test_inventory_filters_agents_projects_and_updated_before(cleanup_environment):
    _index, service = cleanup_environment

    result = service.inventory({
        "agents": ["claude"],
        "projects": ["/project-a"],
        "updated_before": 500,
    })

    assert result["total"] == 1
    assert result["page"][0]["id"] == "c1"


def test_triage_is_idempotent_and_reports_coverage(cleanup_environment):
    _index, service = cleanup_environment
    inventory = service.inventory(None)

    first = service.triage(inventory["scope_id"], [
        {"tool": "claude", "ref": "fsr_c100", "verdict": "delete", "reason": "旧会话"},
        {"tool": "opencode", "ref": "fsr_o100", "verdict": "keep"},
    ])
    repeated = service.triage(inventory["scope_id"], [
        {"tool": "claude", "ref": "fsr_c100", "verdict": "delete"},
    ])

    assert first["covered"] == 2
    assert first["total"] == 4
    assert len(first["remaining_sample"]) == 2
    assert repeated["covered"] == 2
    assert repeated["remaining_sample"][0]["id"] == "c2"


def test_stale_generation_invalidates_scope(cleanup_environment):
    index, service = cleanup_environment
    inventory = service.inventory(None)
    index.generation = 1

    assert service.stale(inventory["scope_id"])
    with pytest.raises(AgentRequestError, match="重新 inventory"):
        service.triage(inventory["scope_id"], [])


def test_invalid_scope_unknown_ref_and_cursor_are_rejected(cleanup_environment):
    _index, service = cleanup_environment
    with pytest.raises(AgentRequestError):
        service.inventory({"unexpected": True})
    with pytest.raises(AgentRequestError):
        service.inventory({"agents": ["missing"]})

    inventory = service.inventory(None)
    with pytest.raises(AgentRequestError):
        service.inventory(None, cursor="not-a-cursor")
    with pytest.raises(AgentRequestError):
        service.triage(inventory["scope_id"], [{
            "tool": "claude",
            "ref": "fsr_missing",
            "verdict": "keep",
        }])
