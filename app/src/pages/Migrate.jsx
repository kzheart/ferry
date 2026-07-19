import { TOOLS, TOOL_NAME, openTerminal } from "../api.js";
import Badge from "../components/Badge.jsx";
import Cmd from "../components/Cmd.jsx";
import LossBlock from "../components/LossBlock.jsx";
import Spin from "../components/Spin.jsx";
import Steps from "../components/Steps.jsx";
import { rpc } from "../api.js";

function Migrate({ mig, setMig, env, onBack, onBackDetail, gotoHistory }) {
  const { meta, ref } = mig;
  const targets = TOOLS.filter(t => t !== meta.tool);

  const dryRun = async dst => {
    setMig(m => ({ ...m, dst, stage: "dry-loading" }));
    try {
      const dry = await rpc("migrate", { src: meta.tool, dst, ref, dry_run: true });
      setMig(m => ({ ...m, dry, stage: "dry" }));
    } catch (e) { setMig(m => ({ ...m, error: e.message, stage: "error" })); }
  };

  const execute = async () => {
    setMig(m => ({ ...m, stage: "running" }));
    try {
      const result = await rpc("migrate", { src: meta.tool, dst: mig.dst, ref });
      setMig(m => ({ ...m, result,
        stage: result.probe && !result.probe.ok ? "failed" : "done" }));
    } catch (e) { setMig(m => ({ ...m, error: e.message, stage: "error" })); }
  };

  const doHandoff = async () => {
    setMig(m => ({ ...m, stage: "handoff-loading" }));
    try {
      const handoff = await rpc("handoff", { src: meta.tool, dst: mig.dst, ref });
      setMig(m => ({ ...m, handoff, stage: "handoff" }));
    } catch (e) { setMig(m => ({ ...m, error: e.message, stage: "error" })); }
  };

  const title = { pick: "选择目标", "dry-loading": "迁移预演", dry: "迁移预演",
    running: "迁移执行中", done: "迁移交付", failed: "迁移降级方案",
    "handoff-loading": "迁移降级方案", handoff: "迁移降级方案" }[mig.stage] || "迁移";

  let body = null;
  if (mig.stage === "pick") {
    body = (
      <div style={{ maxWidth: 640 }}>
        <div style={{ fontSize: 13, fontWeight: 700, marginBottom: 12 }}>选择目标工具</div>
        <div style={{ display: "flex", gap: 12 }}>
          {targets.map(t => {
            const v = (env || {})[t] || {};
            return (
              <button key={t} className="btn card" disabled={!v.installed}
                style={{ flex: 1, flexDirection: "column", padding: 18, gap: 8 }}
                onClick={() => dryRun(t)}>
                <Badge tool={t} /><b>{TOOL_NAME[t]}</b>
                <span className="small muted">
                  {v.installed ? `v${v.version || "?"}${v.verified ? " · 已验证" : " · 未验证,可能降级"}` : "未安装"}
                </span>
              </button>
            );
          })}
        </div>
      </div>
    );
  } else if (mig.stage === "dry-loading" || mig.stage === "handoff-loading") {
    body = <div className="empty"><Spin /> {mig.stage === "dry-loading"
      ? "正在预演转换、计算损耗…" : "正在生成上下文摘要…"}</div>;
  } else if (mig.stage === "dry") {
    const loss = mig.dry.loss;
    const fid = Math.round(100 * loss.native / (loss.native + loss.degrade + loss.drop || 1));
    body = (
      <div className="mig-layout">
        <div className="mig-main"><LossBlock loss={loss} /></div>
        <div className="mig-side">
          <div className="card" style={{ padding: 16 }}>
            <div style={{ fontSize: 13, fontWeight: 700, marginBottom: 12 }}>迁移概要</div>
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              <div className="kv"><span className="k">目标工具</span><span className="v">{TOOL_NAME[mig.dst]}</span></div>
              <div className="kv"><span className="k">保真度</span>
                <span className="v" style={{ color: "#0F9D7A" }}>{fid}% 原生</span></div>
              <div className="kv"><span className="k">源会话</span><span className="v">只读 · 不修改</span></div>
              <div className="kv"><span className="k">会话树</span>
                <span className="v">{mig.dry.tree_count} 个节点</span></div>
              <div className="kv"><span className="k">拓扑保真</span>
                <span className="v" style={{ color: "#0F9D7A" }}>
                  {mig.dry.topology.preserved ? "完整保留" : "存在降级"}</span></div>
              <div className="kv"><span className="k">产物</span><span className="v">新会话副本</span></div>
              <div className="kv"><span className="k">工作目录</span>
                <span className="v mono small">{mig.dry.cwd}</span></div>
            </div>
          </div>
          <label className="confirm">
            <input type="checkbox" checked={mig.confirmed || false}
              onChange={e => setMig(m => ({ ...m, confirmed: e.target.checked }))} />
            <span>我已了解上述 {loss.degrade} 项降级与 {loss.drop} 项丢弃,
              接受以此保真度在 {TOOL_NAME[mig.dst]} 中继续对话。</span>
          </label>
          <button className="btn primary" disabled={!mig.confirmed} onClick={execute}
            style={{ justifyContent: "center", padding: 12 }}>确认并继续迁移 →</button>
          <div className="small muted" style={{ textAlign: "center", lineHeight: 1.5 }}>
            下一步将生成副本并运行探针验收(数十秒,消耗一次极小的模型调用)。</div>
        </div>
      </div>
    );
  } else if (mig.stage === "running") {
    body = <Steps items={[
      ["done", "预演", "损耗已计算并确认"],
      ["run", "生成 + 自动验收", `正在写入 ${TOOL_NAME[mig.dst]} 会话副本并用目标工具真实加载(数十秒)…`],
      ["todo", "交付", "生成接续命令"]]} />;
  } else if (mig.stage === "done") {
    const r = mig.result;
    body = (
      <div style={{ maxWidth: 720, display: "flex", flexDirection: "column", gap: 16 }}>
        <Steps items={[
          ["done", "预演", `原生 ${r.loss.native} · 降级 ${r.loss.degrade} · 丢弃 ${r.loss.drop}`],
          ["done", "生成", `新会话 ${r.session_id} · ${r.tree_count} 个树节点`],
          ["done", "拓扑保真", r.topology.detail],
          ["done", "自动验收", "探针已真实加载会话并得到回复"],
          ["done", "交付", "接续命令已就绪"]]} />
        <div className="card" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <span className="tag ok">验收通过</span>
            <span className="small muted">接续命令出现 = 目标工具已真实加载过该会话</span>
          </div>
          <Cmd text={r.resume} />
          <div style={{ display: "flex", gap: 10 }}>
            <button className="btn primary" onClick={() => openTerminal(r.resume)}>在终端打开</button>
            <button className="btn" onClick={gotoHistory}>查看迁移历史</button>
          </div>
          <div className="small muted">原会话未被修改,随时可回 {TOOL_NAME[meta.tool]} 继续。</div>
        </div>
      </div>
    );
  } else if (mig.stage === "failed") {
    const r = mig.result;
    body = (
      <div style={{ maxWidth: 720, display: "flex", flexDirection: "column", gap: 16 }}>
        <div className="notice bad" style={{ margin: 0 }}>
          迁移未成功,已自动回滚 —— {r.rolled_back ? "临时副本已删除," : ""}原会话未改动。</div>
        <div className="card" style={{ padding: 16 }}>
          <div style={{ fontSize: 13, fontWeight: 700, marginBottom: 8 }}>失败原因(探针输出)</div>
          <div className="mono small selectable" style={{ whiteSpace: "pre-wrap", color: "#566270" }}>
            {r.probe.detail}</div>
        </div>
        <div className="card" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 10 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            <span style={{ fontSize: 14, fontWeight: 700 }}>降级方案 · 使用上下文摘要继续</span>
            <span className="tag warn">非原生迁移</span>
          </div>
          <div className="small" style={{ color: "#5A6672", lineHeight: 1.6 }}>
            把会话浓缩为一份结构化摘要,在 {TOOL_NAME[mig.dst]} 中开启新对话并载入。
            不是逐轮还原,历史细节会丢失。</div>
          <div style={{ display: "flex", gap: 10 }}>
            <button className="btn primary" onClick={doHandoff}>生成摘要方案</button>
            <button className="btn" onClick={onBackDetail}>保持原会话,暂不迁移</button>
          </div>
        </div>
      </div>
    );
  } else if (mig.stage === "handoff") {
    const h = mig.handoff;
    body = (
      <div style={{ maxWidth: 720, display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span style={{ fontSize: 14, fontWeight: 700 }}>摘要已生成</span>
          <span className="tag warn">非原生迁移</span>
        </div>
        <div className="card" style={{ padding: 16 }}>
          <div className="small" style={{ fontWeight: 700, color: "#8A94A0", marginBottom: 8 }}>摘要预览</div>
          <div className="mono small selectable"
            style={{ whiteSpace: "pre-wrap", maxHeight: 300, overflow: "auto", color: "#566270" }}>
            {h.preview}</div>
        </div>
        <div className="card" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 12 }}>
          <div style={{ fontSize: 13, fontWeight: 700 }}>新的开始命令</div>
          <Cmd text={h.command} />
          <div style={{ display: "flex", gap: 10 }}>
            <button className="btn primary" onClick={() => openTerminal(h.command)}>在终端打开</button>
          </div>
        </div>
      </div>
    );
  } else {
    body = <div className="notice bad">出错:{mig.error || "未知错误"}</div>;
  }

  return (
    <div className="page">
      <div className="page-head">
        <div className="crumb">
          <a onClick={onBack}>会话库</a><span style={{ opacity: .5 }}> / </span>
          <a onClick={onBackDetail}>{meta.title || meta.id}</a>
          <span style={{ opacity: .5 }}> / </span>迁移</div>
        <div style={{ display: "flex", alignItems: "center", gap: 14 }}>
          <div className="h1">{title}</div>
          {mig.dst && (
            <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 13, color: "#5A6672" }}>
              <Badge tool={meta.tool} sm />{TOOL_NAME[meta.tool]}
              <span className="muted">→</span>
              <Badge tool={mig.dst} sm />{TOOL_NAME[mig.dst]}
            </div>
          )}
        </div>
        <div className="notice ok">原会话为只读源,迁移不会修改它 —— 生成的是一份可继续对话的副本。</div>
      </div>
      <div className="body">{body}</div>
    </div>
  );
}

export default Migrate;
