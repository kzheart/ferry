from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FRONTEND = ROOT / "app/src"


def test_frontend_uses_shell_platform_shared_and_vertical_modules():
    assert {
        "app",
        "components",
        "modules",
        "platform",
        "shared",
        "shell",
    } <= {
        path.name for path in FRONTEND.iterdir() if path.is_dir()
    }
    assert not (FRONTEND / "api").exists()
    assert not (FRONTEND / "domain").exists()
    assert not (FRONTEND / "modules/shell").exists()
    assert not (FRONTEND / "components/layout").exists()


def test_module_models_live_with_their_consuming_capability():
    assert (FRONTEND / "modules/browser/sessionModel.js").is_file()
    assert (FRONTEND / "modules/browser/sessionAttachment.js").is_file()
    assert (FRONTEND / "modules/askferry/agentChatModel.js").is_file()
    assert (FRONTEND / "modules/askferry/ferryEntities.js").is_file()
    assert (FRONTEND / "modules/overview/overviewModel.js").is_file()
    assert (FRONTEND / "modules/browser/SessionPeekSheet.jsx").is_file()
    app = (FRONTEND / "app/App.jsx").read_text()
    assert "browser/SessionDetail.jsx" not in app
    assert "components/ui/primitives.jsx" not in app


def test_operation_flow_has_one_module_controller():
    controller = FRONTEND / "modules/operations/operationController.ts"
    composition = FRONTEND / "modules/operations/operations.ts"
    assert controller.is_file()
    assert composition.is_file()

    transport = (FRONTEND / "platform/desktop/client.ts").read_text()
    assert "operationApplyAndWait" not in transport

    for relative_path in (
        "app/App.jsx",
        "modules/editing/useSessionEditing.js",
        "modules/migration/MigrateSheet.jsx",
    ):
        source = (FRONTEND / relative_path).read_text()
        assert "operationPlan" not in source
        assert "operationApply" not in source
        assert "operationStatus" not in source
        assert "operationCancel" not in source


def test_session_mutations_live_in_browser_capability():
    app = (FRONTEND / "app/App.jsx").read_text()
    metadata = FRONTEND / "modules/browser/useSessionMetadata.js"
    deletion = FRONTEND / "modules/browser/useSessionDeletion.js"

    assert metadata.is_file()
    assert deletion.is_file()
    assert "operations.plan" not in app
    assert 'engine("session_meta_list")' not in app
    assert "useSessionMetadata" in app
    assert "useSessionDeletion" in app


def test_frontend_core_uses_strict_typescript():
    tsconfig = (ROOT / "app/tsconfig.json").read_text()
    package = (ROOT / "app/package.json").read_text()

    assert '"strict": true' in tsconfig
    assert '"noUncheckedIndexedAccess": true' in tsconfig
    assert '"exactOptionalPropertyTypes": true' in tsconfig
    assert '"typecheck": "tsc --noEmit"' in package
    assert (FRONTEND / "shared/contracts/generated/operations.ts").is_file()
    assert not (FRONTEND / "shared/contracts/generated/operations.js").exists()
    assert (FRONTEND / "platform/desktop/client.ts").is_file()
    assert not (FRONTEND / "api/transport/rpc.js").exists()
