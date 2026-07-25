from engine.adapters.grok.lifecycle import GrokLifecycle


def test_grok_resume_descriptor_uses_structured_id_argument():
    lifecycle = GrokLifecycle()
    lifecycle.executable = "grok"
    descriptor = lifecycle.resume_descriptor(
        "019f0000-0000-7000-8000-000000000000", "/raw/cwd",
    )
    assert descriptor["executable"] == "grok"
    assert descriptor["args"] == [
        "--resume", "019f0000-0000-7000-8000-000000000000",
    ]
    assert lifecycle.delete_undoable is False
