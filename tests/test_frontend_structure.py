import json
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


def test_desktop_webview_has_a_restricted_content_security_policy():
    config = json.loads(
        (ROOT / "app/src-tauri/tauri.conf.json").read_text()
    )
    security = config["app"]["security"]
    csp = security["csp"]
    dev_csp = security["devCsp"]

    assert csp["default-src"] == "'self'"
    assert csp["connect-src"] == "'self' ipc: http://ipc.localhost"
    assert csp["img-src"] == "'self' data: blob:"
    assert csp["object-src"] == "'none'"
    assert csp["frame-src"] == "'none'"
    assert "*" not in " ".join(csp.values())
    assert "localhost:5173" not in " ".join(csp.values())
    assert "ws://localhost:5173" in dev_csp["connect-src"]

    html = (ROOT / "app/index.html").read_text()
    assert '<script type="module" src="/src/themeBootstrap.js">' in html
    assert "localStorage.getItem" not in html


def test_agent_icons_are_resources_not_inline_code():
    """Agent 图标是 assets 资源；新增 Agent 放一个 svg 即可，不必改 icons.jsx。"""
    icons = (FRONTEND / "shared/ui/icons.jsx").read_text(encoding="utf-8")
    assert "import.meta.glob" in icons, "Agent 图标应从 assets/icons 静态收集"
    assert "CLAUDE_PATH" not in icons
    assert "CODEX_PATH" not in icons
    assert "OPENCODE_FRAME_PATH" not in icons

    declared = {
        agent["icon"]
        for agent in json.loads(
            (ROOT / "contracts/agents.json").read_text(encoding="utf-8")
        )["agents"]
    }
    available = {
        path.stem for path in (FRONTEND / "assets/icons").glob("*.svg")
    }
    assert declared <= available, (
        f"契约声明的图标缺少资源: {sorted(declared - available)}"
    )


def test_agent_icon_assets_are_self_contained():
    """svg 自带底色与留白，渲染端零参数；不得引用外部资源。"""
    for path in (FRONTEND / "assets/icons").glob("*.svg"):
        markup = path.read_text(encoding="utf-8")
        assert 'viewBox="0 0 24 24"' in markup, path.name
        assert "<rect" in markup, f"{path.name} 缺少烘焙底色"
        assert "http://www.w3.org/2000/svg" in markup, path.name
        assert "xlink:href" not in markup and "<image" not in markup, path.name
