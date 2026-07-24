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
    assert (FRONTEND / "features/browser/SessionPeekSheet.jsx").is_file()
    app = (FRONTEND / "app/App.jsx").read_text()
    assert "browser/SessionDetail.jsx" not in app
    assert "components/ui/primitives.jsx" not in app


def test_operation_flow_has_one_feature_controller():
    controller = FRONTEND / "features/operations/operationController.js"
    composition = FRONTEND / "features/operations/operations.js"
    assert controller.is_file()
    assert composition.is_file()

    transport = (FRONTEND / "api/transport/rpc.js").read_text()
    assert "operationApplyAndWait" not in transport

    for relative_path in (
        "app/App.jsx",
        "features/editing/useSessionEditing.js",
        "features/migration/MigrateSheet.jsx",
    ):
        source = (FRONTEND / relative_path).read_text()
        assert "operationPlan" not in source
        assert "operationApply" not in source
        assert "operationStatus" not in source
        assert "operationCancel" not in source
