from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FRONTEND = ROOT / "app/src"


def test_frontend_uses_shell_and_vertical_features():
    assert {
        "api",
        "app",
        "components",
        "features",
        "shell",
    } <= {
        path.name for path in FRONTEND.iterdir() if path.is_dir()
    }
    assert not (FRONTEND / "domain").exists()
    assert not (FRONTEND / "features/shell").exists()
    assert not (FRONTEND / "components/layout").exists()


def test_feature_models_live_with_their_consuming_capability():
    assert (FRONTEND / "features/browser/sessionModel.js").is_file()
    assert (FRONTEND / "features/browser/sessionAttachment.js").is_file()
    assert (FRONTEND / "features/askferry/agentChatModel.js").is_file()
    assert (FRONTEND / "features/askferry/ferryEntities.js").is_file()
    assert (FRONTEND / "features/overview/overviewModel.js").is_file()
