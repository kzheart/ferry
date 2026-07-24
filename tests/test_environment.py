from types import SimpleNamespace

from engine.system import environment


class FakePorts:
    def adapters(self):
        return ("claude",)

    def adapter(self, _tool):
        return SimpleNamespace(
            manifest=SimpleNamespace(executables=("claude",)),
        )


def test_environment_reports_executable_state_without_agent_version(monkeypatch):
    monkeypatch.setattr(
        environment.executables,
        "resolve",
        lambda _executable: "/fixture/bin/claude",
    )
    monkeypatch.setattr(
        environment.subprocess,
        "run",
        lambda *_args, **_kwargs: SimpleNamespace(
            returncode=0,
            stdout="claude 999.999.999",
            stderr="",
        ),
    )

    result = environment.inspect(FakePorts())

    assert result == {
        "claude": {
            "installed": True,
            "path": "/fixture/bin/claude",
            "broken": False,
        }
    }
