"""前端结构护栏。

断言的是规则(分层、依赖方向、模型归属、规模上限),不是文件清单:
文件清单会把任何合理重构变成红灯,却挡不住真正的架构违规。
"""
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FRONTEND = ROOT / "app/src"

# 单文件规模上限。超过这个体量的组件基本都在同时承担多个职责。
LINE_LIMIT = 500

# 已知超标文件及其当前规模:允许存在,但只许变小不许变大(棘轮)。
# 降到 LINE_LIMIT 以下后必须从表中删除,避免这张表本身变成陈旧的豁免清单。
KNOWN_OVERSIZED = {
    "modules/browser/SessionRound.jsx": 760,
    "modules/overview/Overview.jsx": 632,
    "shell/AppController.jsx": 585,
}

HORIZONTAL_BUCKETS = {
    "components", "hooks", "models", "utils", "helpers",
    "services", "domain", "common", "core", "lib",
}


def source_files():
    for path in FRONTEND.rglob("*"):
        if path.suffix not in {".js", ".jsx", ".ts", ".tsx"}:
            continue
        if "generated" in path.parts or ".test." in path.name:
            continue
        yield path


def test_no_source_file_grows_past_the_size_limit():
    offenders = []
    for path in source_files():
        relative = str(path.relative_to(FRONTEND))
        lines = len(path.read_text(encoding="utf-8").splitlines())
        ceiling = KNOWN_OVERSIZED.get(relative, LINE_LIMIT)
        if lines > ceiling:
            offenders.append(f"{relative}: {lines} 行 > 上限 {ceiling}")
    assert not offenders, "文件超出规模上限:\n" + "\n".join(offenders)


def test_oversized_ledger_stays_current():
    """已经瘦身达标的文件必须从棘轮表里移除。"""
    stale = []
    for relative, recorded in KNOWN_OVERSIZED.items():
        path = FRONTEND / relative
        assert path.is_file(), f"棘轮表引用了不存在的文件: {relative}"
        lines = len(path.read_text(encoding="utf-8").splitlines())
        assert lines <= recorded, f"{relative} 已增长到 {lines} 行,超过记录的 {recorded}"
        if lines <= LINE_LIMIT:
            stale.append(f"{relative} 已降到 {lines} 行,请从 KNOWN_OVERSIZED 移除")
    assert not stale, "\n".join(stale)


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


def test_modules_are_capabilities_not_horizontal_buckets():
    """modules/ 下只能是产品能力,不能退回按类型分层。"""
    modules = [path for path in (FRONTEND / "modules").iterdir() if path.is_dir()]
    assert modules
    for module in modules:
        assert module.name not in HORIZONTAL_BUCKETS, module.name
        nested = {path.name for path in module.iterdir() if path.is_dir()}
        assert not (nested & HORIZONTAL_BUCKETS), f"{module.name} 内出现横向分层: {nested}"
        assert any(module.glob("*.js")) or any(module.glob("*.jsx")) or any(
            module.glob("*.ts")
        ), f"{module.name} 是空能力目录"


def test_frontend_can_run_component_tests():
    """前端测试必须跑在能渲染组件的运行器上。

    纯函数测试挡不住"某个 prop 忘了往下传"——那类回归只在渲染时暴露,
    而类型检查看不见 props 的传递链路。运行器一旦退回没有 DOM 的形态,
    组件测试会静默消失,所以把这条能力钉成护栏。
    """
    package = json.loads((ROOT / "app/package.json").read_text(encoding="utf-8"))
    dev = package["devDependencies"]

    assert "vitest" in dev
    assert "jsdom" in dev
    assert "@testing-library/react" in dev
    assert package["scripts"]["test"].startswith("vitest")

    config = (ROOT / "app/vitest.config.js").read_text(encoding="utf-8")
    assert 'environment: "jsdom"' in config
    assert "vitest.setup.js" in config

    rendered = sorted(path.name for path in FRONTEND.rglob("*.test.jsx"))
    assert rendered, "组件测试全部消失了"


def test_view_models_live_with_their_consuming_capability():
    """视图模型必须落在能力包内,不得上浮到 shared 或 shell。"""
    for area in ("shared", "shell"):
        for path in (FRONTEND / area).rglob("*"):
            if path.suffix not in {".js", ".ts"} or path.name.endswith(
                (".test.js", ".test.ts")
            ):
                continue
            assert not path.stem.endswith("Model"), (
                f"{path.relative_to(FRONTEND)} 应属于某个能力包"
            )


def test_shell_composes_capabilities_without_reaching_into_their_internals():
    """主壳负责编排:渲染工作区路由与弹层控制器,不直接拼装某个能力的内部视图。"""
    app = (FRONTEND / "shell/AppController.jsx").read_text()
    assert "AppOverlayController" in app
    assert "WorkspaceRouter" in app
    # 详情区视图由 WorkspaceRouter 决定,主壳不直接引入
    assert "browser/SessionDetail.jsx" not in app
    assert "shared/ui/primitives.jsx" not in app
    # 键盘监听归 useAppKeyboardShortcuts,主壳不自己挂全局监听
    assert 'document.addEventListener("keydown"' not in app


def test_deleted_ui_concepts_do_not_return():
    assert not (FRONTEND / "shared/ui/Overlays.jsx").exists()
    session_detail = (FRONTEND / "modules/browser/SessionDetail.jsx").read_text()
    assert "function Round(" not in session_detail
    assert "function ToolCard(" not in session_detail


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
    assert "AGENT_FALLBACK_COLOR" in icons
    assert 'markup?.includes("<rect")' in icons


def test_agent_icon_assets_are_self_contained():
    """彩色资源可烘焙底色，单色资源由渲染端着色；两者都不得引用外部资源。"""
    for path in (FRONTEND / "assets/icons").glob("*.svg"):
        markup = path.read_text(encoding="utf-8")
        assert 'viewBox="0 0 24 24"' in markup, path.name
        assert "<rect" in markup or "currentColor" in markup, (
            f"{path.name} 既没有烘焙底色，也不是可着色的单色资源"
        )
        assert "http://www.w3.org/2000/svg" in markup, path.name
        assert "xlink:href" not in markup and "<image" not in markup, path.name


def test_current_session_filters_follow_capabilities_but_history_keeps_all_tools():
    browser = (FRONTEND / "modules/browser/BrowserOverlays.jsx").read_text()
    history = (FRONTEND / "modules/migration/HistoryOverlays.jsx").read_text()

    assert 'agentsWithCapability("browse")' in browser
    assert "tools.map" in history
    assert "agentsWithCapability" not in history
