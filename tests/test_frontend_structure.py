from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FRONTEND = ROOT / "app/src"


def test_frontend_uses_shell_platform_shared_and_vertical_modules():
    assert {
        "assets",
        "modules",
        "platform",
        "shared",
        "shell",
    } == {
        path.name for path in FRONTEND.iterdir() if path.is_dir()
    }
    assert not (FRONTEND / "api").exists()
    assert not (FRONTEND / "app").exists()
    assert not (FRONTEND / "components").exists()
    assert not (FRONTEND / "domain").exists()
    assert not (FRONTEND / "modules/shell").exists()


def test_module_models_live_with_their_consuming_capability():
    assert (FRONTEND / "modules/browser/sessionModel.js").is_file()
    assert (FRONTEND / "modules/browser/sessionAttachment.js").is_file()
    assert (FRONTEND / "modules/browser/sessionContextMenu.js").is_file()
    assert (FRONTEND / "modules/askferry/agentChatModel.js").is_file()
    assert (FRONTEND / "modules/askferry/agentTimelineModel.js").is_file()
    assert (FRONTEND / "modules/askferry/ferryEntities.js").is_file()
    assert (FRONTEND / "modules/askferry/AgentWorkflowCards.jsx").is_file()
    assert (FRONTEND / "modules/askferry/AgentMenus.jsx").is_file()
    assert (FRONTEND / "modules/askferry/AgentComposer.jsx").is_file()
    assert (FRONTEND / "modules/askferry/AgentChatItem.jsx").is_file()
    assert (FRONTEND / "modules/askferry/AgentToolTrace.jsx").is_file()
    assert (FRONTEND / "modules/overview/overviewModel.js").is_file()
    assert (FRONTEND / "modules/browser/SessionPeekSheet.jsx").is_file()
    assert (FRONTEND / "modules/browser/SessionImagePreview.jsx").is_file()
    assert (FRONTEND / "modules/browser/SessionContext.jsx").is_file()
    assert (FRONTEND / "modules/browser/PendingEditBar.jsx").is_file()
    assert (FRONTEND / "modules/browser/SessionRound.jsx").is_file()
    assert (FRONTEND / "modules/browser/BrowserOverlays.jsx").is_file()
    assert (FRONTEND / "modules/editing/EditOverlays.jsx").is_file()
    assert (FRONTEND / "modules/migration/HistoryOverlays.jsx").is_file()
    assert (FRONTEND / "modules/onboarding/Guide.jsx").is_file()
    assert (FRONTEND / "modules/onboarding/useOnboarding.js").is_file()
    assert (FRONTEND / "shell/AppOverlays.jsx").is_file()
    assert (FRONTEND / "shell/AppOverlayController.jsx").is_file()
    assert (FRONTEND / "shell/SearchPalette.jsx").is_file()
    assert (FRONTEND / "shell/useAppKeyboardShortcuts.js").is_file()
    assert (FRONTEND / "shell/useResourcePaneLayout.js").is_file()
    assert not (FRONTEND / "shared/ui/Overlays.jsx").exists()
    app = (FRONTEND / "shell/AppController.jsx").read_text()
    assert "browser/SessionDetail.jsx" not in app
    assert "shared/ui/primitives.jsx" not in app
    assert "shared/ui/Overlays.jsx" not in app
    assert "AppOverlayController" in app
    assert "document.addEventListener(\"keydown\"" not in app
    assert app.index("metadata: metaMap") < app.index("useLibraryResourcePane({")

    session_detail = (
        FRONTEND / "modules/browser/SessionDetail.jsx"
    ).read_text()
    assert "function Round(" not in session_detail
    assert "function ToolCard(" not in session_detail
    assert "SessionRound" in session_detail

    tool_trace = (FRONTEND / "modules/askferry/AgentToolTrace.jsx").read_text()
    workflow_cards = (FRONTEND / "modules/askferry/AgentWorkflowCards.jsx").read_text()
    assert "EntityCards" in tool_trace
    assert "onNavigate={onNavigate}" in tool_trace
    assert "entitiesFromToolResult" in workflow_cards
    assert "EntityCards" in workflow_cards


def test_operation_flow_has_one_module_controller():
    controller = FRONTEND / "modules/operations/operationController.ts"
    composition = FRONTEND / "modules/operations/operations.ts"
    assert controller.is_file()
    assert composition.is_file()

    transport = (FRONTEND / "platform/desktop/client.ts").read_text()
    assert "operationApplyAndWait" not in transport

    for relative_path in (
        "shell/AppController.jsx",
        "modules/editing/useSessionEditing.js",
        "modules/migration/MigrateSheet.jsx",
    ):
        source = (FRONTEND / relative_path).read_text()
        assert "operationPlan" not in source
        assert "operationApply" not in source
        assert "operationStatus" not in source
        assert "operationCancel" not in source


def test_session_mutations_live_in_browser_capability():
    app = (FRONTEND / "shell/AppController.jsx").read_text()
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
