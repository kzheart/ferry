import json

from engine.server import cli


def test_environment_command_uses_the_engine_capability_facade(
    monkeypatch,
    capsys,
):
    class Application:
        closed = False

        def environment(self):
            return {"environment": "current"}

        def close(self):
            self.closed = True

    application = Application()
    monkeypatch.setattr(cli, "build_engine", lambda: application)

    cli.main(["env"])

    assert json.loads(capsys.readouterr().out) == {
        "environment": "current",
    }
    assert application.closed is True
