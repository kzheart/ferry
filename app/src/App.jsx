import { useEffect, useMemo, useRef, useState } from "react";
import {
  rpc, openTerminal, TOOL_NAME, TOOL_SHORT, BIG,
  fmtSize, fmtTime, resumeCommand,
} from "./api.js";

const TOOLS = ["claude", "codex", "opencode"];

/* ---------- 小组件 ---------- */

const Spin = () => <span className="spin" />;
const Badge = ({ tool, sm }) => (
  <div className={`badge ${tool} ${sm ? "sm" : ""}`}>{TOOL_SHORT[tool]}</div>
);

function CopyBtn({ text, className = "copy" }) {
  const [ok, setOk] = useState(false);
  return (
    <button className={className} onClick={() => {
      navigator.clipboard.writeText(text);
      setOk(true); setTimeout(() => setOk(false), 1200);
    }}>{ok ? "已复制" : "复制"}</button>
  );
}

const Cmd = ({ text }) => (
  <div className="cmd mono">
    <span className="c selectable">{text}</span>
    <CopyBtn text={text} />
  </div>
);

function Steps({ items }) {
  return (
    <div className="steps">
      {items.map(([st, t, d], i) => (
        <div className="step" key={i}>
          <div className="rail">
            <span className={`ico ${st}`}>
              {st === "done" ? "✓" : st === "run" ? <Spin /> : st === "fail" ? "✕" : ""}
            </span>
            {i < items.length - 1 && <span className="line" />}
          </div>
          <div className="txt"><div className="t">{t}</div><div className="d">{d}</div></div>
        </div>
      ))}
    </div>
  );
}

function LossBlock({ loss }) {
  const total = loss.native + loss.degrade + loss.drop || 1;
  return (
    <>
      <div style={{ fontSize: 13, fontWeight: 700 }}>
        损耗报告 · 共 {loss.native + loss.degrade + loss.drop} 项
        {loss.degrade + loss.drop === 0 &&
          <span className="tag ok" style={{ marginLeft: 8 }}>无损</span>}
      </div>
      <div className="loss-bar">
        <div style={{ flex: loss.native / total, background: "#17A886" }} />
        <div style={{ flex: loss.degrade / total, background: "#D99A2B" }} />
        <div style={{ flex: loss.drop / total, background: "#CB5A52" }} />
      </div>
      <div className="loss-cards">
        {[["native", "原生映射", loss.native, "直接对应目标结构", "#0F9D7A", "#0B7A5E"],
          ["degrade", "降级为文本", loss.degrade, "保留内容,失去结构", "#A66A00", "#8A5A00"],
          ["drop", "丢弃", loss.drop, "无法安全迁移", "#C2413A", "#9E332D"]]
          .map(([k, lbl, n, d, c1, c2]) => (
            <div className="card loss-card" key={k}>
              <div className="lbl" style={{ color: c1 }}><span className={`sq ${k}`} />{lbl}</div>
              <div className="n" style={{ color: c2 }}>{n}</div>
              <div className="d">{d}</div>
            </div>
          ))}
      </div>
      {(loss.degrade_details.length > 0 || loss.drop_details.length > 0) && (
        <div className="card" style={{ padding: "13px 15px", fontSize: 12, color: "#5A6672",
          display: "flex", flexDirection: "column", gap: 7 }}>
          {loss.degrade_details.map((x, i) =>
            <div key={"d" + i}><span className="sq degrade" style={{ marginRight: 7 }} />{x}</div>)}
          {loss.drop_details.map((x, i) =>
            <div key={"x" + i}><span className="sq drop" style={{ marginRight: 7 }} />{x}</div>)}
        </div>
      )}
    </>
  );
}

/* ---------- 信任面板 ---------- */

function TrustPanel({ env }) {
  return (
    <div className="trust">
      <div className="trust-title">环境与信任</div>
      <div className="trust-body">
        {env ? TOOLS.map(k => {
          const v = env[k] || {};
          const dot = v.installed ? (v.verified ? "ok" : "warn") : "miss";
          const ver = !v.installed ? "未安装"
            : `v${v.version || "?"}${v.verified ? " · 已验证" : ""}`;
          return (
            <div key={k}>
              <div className="trust-row">
                <span className={`dot ${dot}`} />
                <span className="name">{TOOL_NAME[k]}</span>
                <span className="ver">{ver}</span>
              </div>
              {v.installed && !v.verified &&
                <div className="trust-warn">与黄金样本 {v.golden || "—"} 不一致 · 建议自检</div>}
            </div>
          );
        }) : <span className="small muted">检测中…</span>}
      </div>
      <div className="trust-foot">版本漂移时自动降级迁移策略。</div>
    </div>
  );
}

/* ---------- 会话库 / 首次启动 ---------- */

function First({ env, onScan }) {
  return (
    <div className="page"><div className="first">
      <div className="logo" style={{ width: 44, height: 44, fontSize: 20, borderRadius: 11 }}>S</div>
      <div className="big">尚未扫描到任何会话</div>
      <div className="desc">Session Bridge 会读取本机 CLI 工具的会话记录,统一浏览、迁移与编辑。
        所有读取与处理都在本地完成,不上传任何会话内容。</div>
      <div className="tools">
        {TOOLS.map(k => {
          const v = (env || {})[k] || {};
          return (
            <div className="card tool-card" key={k}>
              <div style={{ display: "flex", justifyContent: "center", marginBottom: 8 }}><Badge tool={k} /></div>
              <div style={{ fontWeight: 600, fontSize: 13 }}>{TOOL_NAME[k]}</div>
              <div className="small muted" style={{ marginTop: 4 }}>
                {v.installed ? `已安装 · v${v.version || "?"}` : "未检测到安装"}</div>
              <div style={{ marginTop: 6 }}>
                {v.installed ? <span className="tag ok">可扫描</span>
                  : <span className="small muted">跳过</span>}</div>
            </div>
          );
        })}
      </div>
      <button className="btn primary" style={{ padding: "11px 26px", fontSize: 14 }}
        onClick={onScan}>开始扫描</button>
      <div className="hint">迁移或还原后的<b>探针验收</b>会真实加载会话并进行一次极小的模型调用,
        仅用于验证可用性。</div>
    </div></div>
  );
}

function Library({ scan, scanning, env, onScan, onOpen }) {
  const [q, setQ] = useState("");
  const [tf, setTf] = useState(new Set());

  if (!scan && !scanning) return <First env={env} onScan={onScan} />;
  if (!scan) {
    return <div className="page"><div className="first">
      <span className="spin" style={{ width: 22, height: 22 }} />
      <div className="big">正在扫描本机会话…</div>
      <div className="desc">首次扫描会解析全部历史文件,之后走缓存,只需零点几秒。</div>
    </div></div>;
  }

  const list = scan.sessions.filter(s => {
    if (tf.size && !tf.has(s.tool)) return false;
    if (q) return (s.title + s.dir + s.id).toLowerCase().includes(q.toLowerCase());
    return true;
  });

  const chip = k => {
    const info = scan.tools[k] || {};
    const v = (env || {})[k] || {};
    let cls = "ok", t = TOOL_NAME[k], d = `扫描完成 · 找到 ${info.count ?? 0} 个会话`;
    if (!v.installed && !info.count) { cls = "miss"; d = "未安装 · 不纳入结果"; }
    else if (info.ok === false) { cls = "warn"; d = `扫描出错:${info.error || ""}`; }
    return (
      <div className="scanchip" key={k}>
        <span className={`bigdot ${cls}`} />
        <div><div className="t" style={cls === "miss" ? { color: "#78828D" } : {}}>{t}</div>
          <div className="d">{d}</div></div>
      </div>
    );
  };

  return (
    <div className="page">
      <div className="page-head">
        <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between" }}>
          <div>
            <div className="h1">会话库</div>
            <div className="sub">统一汇聚 Claude Code、Codex CLI、OpenCode 的本机会话</div>
          </div>
          <button className="btn" onClick={onScan} disabled={scanning}>
            {scanning ? <><Spin />扫描中</> : "重新扫描"}</button>
        </div>
        <div className="toolbar">
          <div className="search">
            <span className="muted">🔍</span>
            <input placeholder="搜索标题、目录或会话 ID…" value={q}
              onChange={e => setQ(e.target.value)} />
          </div>
          {TOOLS.map(k => (
            <button key={k} className={`filter ${tf.has(k) ? "on" : ""}`}
              onClick={() => {
                const n = new Set(tf);
                n.has(k) ? n.delete(k) : n.add(k);
                setTf(n);
              }}>{TOOL_NAME[k]}</button>
          ))}
        </div>
        <div className="scanbar">{TOOLS.map(chip)}</div>
      </div>
      <div className="body">
        <div className="grid-head">
          <div>工具</div><div>标题 / 目录</div><div>活跃时间</div>
          <div style={{ textAlign: "right" }}>消息数</div>
          <div style={{ textAlign: "right" }}>体积</div><div />
        </div>
        <div className="rows">
          {list.slice(0, 300).map(s => (
            <div className="row" key={s.tool + s.id} onClick={() => onOpen(s)}>
              <Badge tool={s.tool} />
              <div style={{ minWidth: 0 }}>
                <div className="title">{s.title || "(无标题)"}</div>
                <div className="dir mono">{s.dir || s.id}</div>
              </div>
              <div style={{ fontSize: 12.5, color: "#5A6672" }}>{fmtTime(s.updated)}</div>
              <div className="num">{s.count}</div>
              <div className="num">{fmtSize(s.size)}</div>
              <div className="muted">›</div>
            </div>
          ))}
          {list.length === 0 && <div className="empty">没有匹配的会话</div>}
        </div>
        <div className="foot">
          <span>显示 {Math.min(list.length, 300)} / {list.length} 个会话</span>
          <span>{env && !env.opencode.installed ? "OpenCode 未安装,其会话不在结果中" : ""}</span>
        </div>
      </div>
    </div>
  );
}

/* ---------- 会话详情 ---------- */

function ToolCallCard({ b }) {
  const [open, setOpen] = useState(false);
  const big = b.size > BIG;
  const label = b.op
    ? `${b.name} · ${String(typeof b.input === "object"
        ? (b.input.command || b.input.file_path || "") : "").slice(0, 80)}`
    : b.name;
  return (
    <div className={`toolcall ${big ? "big" : ""}`}>
      <div className="tool-head" onClick={() => setOpen(!open)}>
        <span className="tool-tag mono">工具调用</span>
        <span className="tool-name mono">{label}</span>
        <span style={{ flex: 1 }} />
        {big && <span className="small" style={{ color: "#A66A00", fontWeight: 600 }}>大输出</span>}
        <span className="small muted">{fmtSize(b.size)} · {open ? "折叠" : "展开"}</span>
      </div>
      {open && (
        <div className="tool-out mono selectable">
          {typeof b.input === "object"
            ? JSON.stringify(b.input, null, 1).slice(0, 2000)
            : String(b.input).slice(0, 2000)}
          {"\n———— 输出 ————\n"}
          {(b.output || "(无输出)").slice(0, 200000)}
        </div>
      )}
    </div>
  );
}

function MessageView({ msg }) {
  const texts = msg.blocks.filter(b => b.kind === "text" && b.text.trim());
  const tools = msg.blocks.filter(b => b.kind === "tool");
  return (
    <>
      {texts.length > 0 && (
        <div className="msg">
          <div className={`avatar ${msg.role}`}>{msg.role === "user" ? "你" : "AI"}</div>
          <div className={`bubble ${msg.role}`}>
            <div className="bubble-head">
              <span className="who">{msg.role === "user" ? "用户消息" : "AI 回复"}</span>
              <span className="sz">{fmtSize(texts.reduce((a, b) => a + b.size, 0))}</span>
            </div>
            <div className="text selectable">{texts.map(t => t.text).join("\n")}</div>
          </div>
        </div>
      )}
      {tools.map((b, i) => <ToolCallCard b={b} key={i} />)}
    </>
  );
}

function Detail({ detail, env, onBack, onMigrate, onEdit }) {
  const { meta, data, error } = detail;
  const resume = resumeCommand(meta.tool, meta.id, meta.dir);
  return (
    <div className="page">
      <div className="detail-head">
        <div className="crumb"><a onClick={onBack}>会话库</a>
          <span style={{ opacity: .5 }}> / </span>{meta.title || meta.id}</div>
        <div className="detail-title-row">
          <div className="detail-title">
            <Badge tool={meta.tool} />
            <div>
              <div className="h">{meta.title || "(无标题)"}</div>
              <div className="dir mono">{meta.dir || ""}</div>
            </div>
          </div>
          <div style={{ display: "flex", gap: 8, flex: "none" }}>
            <button className="btn primary" onClick={onMigrate}>迁移 →</button>
            {meta.tool === "claude" &&
              <button className="btn" onClick={onEdit} disabled={!data}>会话编辑</button>}
            <CopyBtn text={resume} className="btn" />
          </div>
        </div>
        <div className="detail-meta">
          <span>来源 · <b>{TOOL_NAME[meta.tool]}</b></span>
          <span>更新 · {fmtTime(meta.updated)}</span>
          <span>{meta.count} 条消息</span>
          <span>{fmtSize(meta.size)}</span>
          {data && data.loss.length > 0 &&
            <span className="chip drop">读取损耗 {data.loss.length} 项</span>}
        </div>
      </div>
      <div className="body">
        {error ? <div className="empty">读取失败:{error}</div>
          : !data ? <div className="empty"><Spin /> 解析会话中…</div>
          : <div className="timeline">{data.messages.map(m => <MessageView msg={m} key={m.index} />)}</div>}
      </div>
    </div>
  );
}

/* ---------- 迁移 ---------- */

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
          ["done", "生成", `新会话 ${r.session_id}`],
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

/* ---------- 会话编辑 ---------- */

function editTurns(data) {
  // 轮 = 一条用户消息及其后的全部 AI 回复与工具调用
  const turns = [];
  let cur = null;
  for (const m of data.messages) {
    if (m.role === "user" || !cur) { cur = { msgs: [] }; turns.push(cur); }
    cur.msgs.push(m);
  }
  return turns;
}

function RewriteModal({ data, initial, onClose, onOk }) {
  const cands = data.messages.filter(m => m.uuid &&
    m.blocks.some(b => b.kind === "text" && b.text.trim()));
  const [idx, setIdx] = useState(0);
  const textOf = m => m.blocks.find(b => b.kind === "text").text;
  const [text, setText] = useState(cands.length ? textOf(cands[0]) : "");
  return (
    <div className="overlay">
      <div className="modal">
        <div className="modal-head"><span className="t">改写单条消息</span>
          <button className="x" onClick={onClose}>✕</button></div>
        <div className="modal-body">
          <label className="f">选择消息</label>
          <select value={idx} onChange={e => {
            const i = +e.target.value; setIdx(i); setText(textOf(cands[i]));
          }}>
            {cands.map((m, i) => (
              <option value={i} key={m.uuid}>
                #{m.index} {m.role === "user" ? "用户" : "助手"} · {textOf(m).slice(0, 60)}
              </option>
            ))}
          </select>
          <label className="f">新文本</label>
          <textarea rows={6} value={text} onChange={e => setText(e.target.value)} />
        </div>
        <div className="modal-foot">
          <button className="btn" onClick={onClose}>取消</button>
          <button className="btn primary" disabled={!cands.length}
            onClick={() => onOk({ uuid: cands[idx].uuid, text })}>暂存改写</button>
        </div>
      </div>
    </div>
  );
}

function Edit({ edit, setEdit, onBack, onBackDetail, gotoSnapshots, afterApply }) {
  const { meta, ref, data } = edit;
  const [modal, setModal] = useState(false);
  const turns = useMemo(() => editTurns(data), [data]);

  const ops = useMemo(() => {
    const o = [...edit.delTurns].sort((a, b) => b - a)
      .map(t => ({ op: "delete-turn", turn: t + 1 }));
    if (edit.truncate) o.push({ op: "truncate", threshold: edit.threshold });
    if (edit.rewrite) o.push({ op: "rewrite", uuid: edit.rewrite.uuid, text: edit.rewrite.text });
    return o;
  }, [edit]);

  const preview = async () => {
    setEdit(e => ({ ...e, preview: "loading" }));
    try {
      const p = await rpc("edit_preview", { ref, ops });
      setEdit(e => ({ ...e, preview: p }));
    } catch (err) { setEdit(e => ({ ...e, preview: { error: err.message } })); }
  };

  const apply = async () => {
    setEdit(e => ({ ...e, applying: true }));
    try {
      const result = await rpc("edit_apply", { ref, ops });
      setEdit(e => ({ ...e, applying: false, result }));
      afterApply();
    } catch (err) {
      setEdit(e => ({ ...e, applying: false, result: { ok: false, error: err.message } }));
    }
  };

  if (edit.result) {
    const r = edit.result;
    return (
      <div className="page">
        <div className="page-head">
          <div className="crumb"><a onClick={onBack}>会话库</a>
            <span style={{ opacity: .5 }}> / </span>会话编辑</div>
          <div className="h1">{r.ok ? "编辑完成" : "编辑未生效"}</div>
        </div>
        <div className="body">
          <div style={{ maxWidth: 680, display: "flex", flexDirection: "column", gap: 16 }}>
            {r.ok ? (
              <>
                <div className="notice ok" style={{ margin: 0 }}>
                  已应用:{(r.notes || []).join(" · ")} · 探针验收通过</div>
                <div className="card" style={{ padding: 18, display: "flex", flexDirection: "column", gap: 12 }}>
                  <div className="small muted">快照:<span className="mono">{r.snapshot}</span>
                    (可在「快照与还原」回退)</div>
                  <Cmd text={r.resume} />
                  <div style={{ display: "flex", gap: 10 }}>
                    <button className="btn primary" onClick={() => openTerminal(r.resume)}>在终端打开</button>
                    <button className="btn" onClick={gotoSnapshots}>查看快照</button>
                  </div>
                </div>
              </>
            ) : (
              <>
                <div className="notice bad" style={{ margin: 0 }}>
                  {r.error || "失败"}{r.snapshot ? " · 已自动还原快照,会话保持原状" : ""}</div>
                {r.probe && <div className="card" style={{ padding: 16 }}>
                  <div className="small mono selectable" style={{ whiteSpace: "pre-wrap" }}>
                    {r.probe.detail}</div></div>}
              </>
            )}
            <div><button className="btn" onClick={onBack}>返回会话库</button></div>
          </div>
        </div>
      </div>
    );
  }

  const pv = edit.preview;
  return (
    <div className="page">
      <div className="detail-head" style={{ paddingBottom: 14 }}>
        <div className="crumb">
          <a onClick={onBack}>会话库</a><span style={{ opacity: .5 }}> / </span>
          <a onClick={onBackDetail}>{meta.title || meta.id}</a>
          <span style={{ opacity: .5 }}> / </span>会话编辑</div>
        <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
          <div className="h1" style={{ fontSize: 21 }}>会话编辑</div>
          <div className="small muted">点击轮次标记删除 · 右侧配置裁剪与改写</div>
        </div>
      </div>
      <div className="edit-layout">
        <div className="edit-list">
          <div className="small" style={{ fontWeight: 700, color: "#8A94A0" }}>
            多轮历史 · 共 {turns.length} 轮,点击选中/取消删除</div>
          {turns.map((t, i) => {
            const del = edit.delTurns.has(i);
            const first = t.msgs[0].blocks.find(b => b.kind === "text");
            const sz = t.msgs.reduce((a, mm) =>
              a + mm.blocks.reduce((x, b) => x + (b.size || 0), 0), 0);
            const tools = t.msgs.flatMap(mm => mm.blocks.filter(b => b.kind === "tool"));
            const bigTool = tools.some(b => b.size > BIG);
            return (
              <div className={`turn ${del ? "del" : ""}`} key={i}
                onClick={() => setEdit(e => {
                  const n = new Set(e.delTurns);
                  n.has(i) ? n.delete(i) : n.add(i);
                  return { ...e, delTurns: n, preview: null };
                })}>
                <span className="cb">{del ? "✓" : ""}</span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div className="t">第 {i + 1} 轮 · 用户 → 助手{del ? " · 已标记删除" : ""}</div>
                  <div className="d">{(first ? first.text : "(工具轮)").slice(0, 90)}</div>
                  {tools.length > 0 && (
                    <div className="d">{tools.length} 次工具调用
                      {bigTool && <span style={{ color: "#A66A00", fontWeight: 600 }}> · 含大输出</span>}
                    </div>
                  )}
                </div>
                <span className="small muted">{fmtSize(sz)}</span>
              </div>
            );
          })}
        </div>
        <div className="edit-side">
          <div style={{ fontSize: 13, fontWeight: 700 }}>已暂存 {ops.length} 项操作</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 7, fontSize: 12, color: "#5A6672" }}>
            {[...edit.delTurns].sort((a, b) => a - b).map(t =>
              <div key={t}><span className="sq drop" style={{ marginRight: 7 }} />删除轮次 · 第 {t + 1} 轮</div>)}
            {edit.truncate &&
              <div><span className="sq degrade" style={{ marginRight: 7 }} />裁剪超过 {edit.threshold} 字符的工具输出</div>}
            {edit.rewrite &&
              <div><span className="sq native" style={{ marginRight: 7 }} />改写 1 条消息</div>}
            {ops.length === 0 && <div className="muted">尚未暂存任何操作</div>}
          </div>
          <div className="card" style={{ padding: 13, display: "flex", flexDirection: "column", gap: 9 }}>
            <label className="confirm" style={{ fontSize: 12.5 }}>
              <input type="checkbox" checked={edit.truncate}
                onChange={e => setEdit(x => ({ ...x, truncate: e.target.checked, preview: null }))} />
              <span>裁剪超大工具输出</span>
            </label>
            <div>
              <label className="f">保留阈值(字符)</label>
              <input type="number" min={256} step={256} value={edit.threshold}
                onChange={e => setEdit(x => ({ ...x, threshold: +e.target.value || 4096, preview: null }))} />
            </div>
          </div>
          <div className="card" style={{ padding: 13, display: "flex", flexDirection: "column", gap: 9 }}>
            <div className="f" style={{ margin: 0 }}>改写单条消息</div>
            <button className="btn" onClick={() => setModal(true)}>
              {edit.rewrite ? "重新选择消息…" : "选择消息并改写…"}</button>
          </div>
          <button className="btn" disabled={!ops.length} onClick={preview}>计算差异预览</button>
          {pv === "loading" && <div className="small muted"><Spin /> 计算中…</div>}
          {pv && pv !== "loading" && !pv.error && (
            <div className="card" style={{ padding: 14, display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="kv"><span className="k">消息数</span>
                <span className="v">{pv.before.count} <span className="muted">→</span> <b>{pv.after.count}</b></span></div>
              <div className="kv"><span className="k">总体积</span>
                <span className="v">{fmtSize(pv.before.size)} <span className="muted">→</span> <b>{fmtSize(pv.after.size)}</b>
                  <span style={{ color: "#0F9D7A", fontWeight: 600 }}> −{fmtSize(pv.before.size - pv.after.size)}</span></span></div>
              <div className="small muted">{pv.notes.join(" · ")}</div>
            </div>
          )}
          {pv && pv.error && <div className="notice bad" style={{ margin: 0 }}>{pv.error}</div>}
          <div style={{ flex: 1 }} />
          <div className="small muted" style={{ lineHeight: 1.5 }}>
            应用前会自动创建快照;若探针验收未通过将自动还原,绝不留下坏会话。</div>
          <button className="btn primary" disabled={!ops.length || edit.applying} onClick={apply}
            style={{ justifyContent: "center", padding: 12 }}>
            {edit.applying ? <><Spin /> 应用并验收中(数十秒)…</> : "应用编辑(快照保护)"}</button>
        </div>
      </div>
      {modal && <RewriteModal data={data} onClose={() => setModal(false)}
        onOk={rw => { setEdit(e => ({ ...e, rewrite: rw, preview: null })); setModal(false); }} />}
    </div>
  );
}

/* ---------- 迁移历史 ---------- */

function History() {
  const [rows, setRows] = useState(null);
  const [why, setWhy] = useState(null);
  useEffect(() => { rpc("history").then(setRows).catch(() => setRows([])); }, []);
  const cols = "150px 1fr 210px 110px 130px";
  return (
    <div className="page">
      <div className="page-head">
        <div className="h1">迁移历史</div>
        <div className="sub">每一次转出记录都在这里 —— 上次迁移生成的会话去了哪,一目了然</div>
      </div>
      <div className="body">
        {!rows ? <div className="empty"><Spin /></div>
          : rows.length === 0 ? <div className="empty">还没有迁移记录。从会话库选择一个会话开始迁移。</div>
          : (
            <div className="table">
              <div className="trow head" style={{ gridTemplateColumns: cols }}>
                <div>时间</div><div>源会话 → 目标</div><div>损耗摘要</div><div>探针验收</div><div>操作</div>
              </div>
              {rows.map((r, i) => {
                const ok = r.probe && r.probe.ok;
                const failed = r.probe && !r.probe.ok;
                return (
                  <div className="trow" style={{ gridTemplateColumns: cols }} key={i}>
                    <div className="muted">{fmtTime(r.time)}</div>
                    <div style={{ minWidth: 0 }}>
                      <div style={{ fontWeight: 600, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {r.title || r.source_id}</div>
                      <div className="small muted">→ {TOOL_NAME[r.dst]} · <span className="mono">
                        {failed ? "未保留产物" : (r.session_id || "").slice(0, 20)}</span></div>
                    </div>
                    <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                      <span className="chip native">原生 {r.loss.native}</span>
                      <span className="chip degrade">降级 {r.loss.degrade}</span>
                      <span className="chip drop">丢弃 {r.loss.drop}</span>
                    </div>
                    <div>{ok ? <span className="tag ok">验收通过</span>
                      : failed ? <span className="tag bad">失败 · 已回滚</span>
                      : <span className="tag warn">未验收</span>}</div>
                    <div style={{ display: "flex", gap: 12 }}>
                      {ok && <CopyBtn text={r.resume} className="btn"
                        style={{ padding: "4px 10px" }} />}
                      {failed && <a onClick={() => setWhy(r)}>查看原因</a>}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        <div className="foot"><span>失败的迁移不会留下任何产物;源会话始终保持只读、可继续使用。</span></div>
      </div>
      {why && (
        <div className="overlay" onClick={() => setWhy(null)}>
          <div className="modal" onClick={e => e.stopPropagation()}>
            <div className="modal-head"><span className="t">失败原因(探针输出)</span>
              <button className="x" onClick={() => setWhy(null)}>✕</button></div>
            <div className="modal-body">
              <div className="mono small selectable" style={{ whiteSpace: "pre-wrap" }}>
                {why.probe ? why.probe.detail : "未知"}</div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/* ---------- 快照与还原 ---------- */

function Snapshots() {
  const [rows, setRows] = useState(null);
  const [busy, setBusy] = useState(null);     // path of snapshot being restored
  const [msg, setMsg] = useState(null);
  const load = () => rpc("snapshots").then(setRows).catch(() => setRows([]));
  useEffect(() => { load(); }, []);
  const cols = "150px 1fr 110px 180px";
  const total = (rows || []).reduce((a, r) => a + r.size, 0);

  const restore = async r => {
    if (!confirm(`还原会话 ${r.session} 到 ${fmtTime(r.time)} 的快照?\n还原后会运行探针验收(数十秒)。`)) return;
    setBusy(r.path);
    try {
      const res = await rpc("snapshot_restore", { session: r.session });
      setMsg(res.ok ? "还原成功,探针验收通过。" : `还原未生效:${res.error || ""}`);
    } catch (e) { setMsg(`还原失败:${e.message}`); }
    setBusy(null); load();
  };

  const del = async r => {
    if (!confirm("删除该快照?此操作不可恢复。")) return;
    await rpc("snapshot_delete", { path: r.path });
    load();
  };

  return (
    <div className="page">
      <div className="page-head">
        <div className="h1">快照与还原</div>
        <div className="sub">每次编辑前自动创建快照 · 可还原或清理 · 还原经探针验收</div>
      </div>
      <div className="body">
        {msg && <div className={`notice ${msg.includes("成功") ? "ok" : "bad"}`}
          style={{ marginTop: 0, marginBottom: 12 }}>{msg}
          <a style={{ marginLeft: 8 }} onClick={() => setMsg(null)}>关闭</a></div>}
        {!rows ? <div className="empty"><Spin /></div>
          : rows.length === 0 ? <div className="empty">还没有快照。编辑会话时会自动创建。</div>
          : (
            <>
              <div className="small muted" style={{ marginBottom: 10 }}>
                共 {rows.length} 个 · 占用 {fmtSize(total)}</div>
              <div className="table">
                <div className="trow head" style={{ gridTemplateColumns: cols }}>
                  <div>时间</div><div>所属会话</div><div>大小</div><div>操作</div>
                </div>
                {rows.map(r => (
                  <div className="trow" style={{ gridTemplateColumns: cols }} key={r.path}>
                    <div className="muted">{fmtTime(r.time)}</div>
                    <div className="mono small">{r.session}</div>
                    <div>{fmtSize(r.size)}</div>
                    <div style={{ display: "flex", gap: 14 }}>
                      {busy === r.path
                        ? <span className="small muted"><Spin /> 还原并验收中…</span>
                        : <>
                            <a onClick={() => restore(r)}>还原</a>
                            <a style={{ color: "#9E332D" }} onClick={() => del(r)}>清理</a>
                          </>}
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
        <div className="foot">
          <span>还原会先保住现状;探针验收失败时保持当前状态不变,绝不留下半还原的会话。</span></div>
      </div>
    </div>
  );
}

/* ---------- 根组件 ---------- */

export default function App() {
  const [view, setView] = useState("library");
  const [env, setEnv] = useState(null);
  const [scan, setScan] = useState(null);
  const [scanning, setScanning] = useState(false);
  const [detail, setDetail] = useState(null);
  const [mig, setMig] = useState(null);
  const [edit, setEdit] = useState(null);
  const scanned = useRef(false);

  const doScan = async () => {
    setScanning(true);
    try { setScan(await rpc("scan")); }
    catch (e) { setScan({ tools: {}, sessions: [], error: e.message }); }
    setScanning(false);
  };

  useEffect(() => {
    rpc("env").then(setEnv).catch(() => {});
    if (!scanned.current) { scanned.current = true; doScan(); }
  }, []);

  const openDetail = async s => {
    const ref = s.tool === "opencode" ? s.id : (s.path || s.id);
    setDetail({ meta: s, ref, data: null });
    setView("detail");
    try {
      const data = await rpc("show", { tool: s.tool, ref });
      setDetail(d => d && d.meta.id === s.id ? { ...d, data } : d);
    } catch (e) {
      setDetail(d => d && d.meta.id === s.id ? { ...d, error: e.message } : d);
    }
  };

  const nav = [["library", "会话库"], ["history", "迁移历史"], ["snapshots", "快照与还原"]];
  const navOn = v => v === view ||
    (v === "library" && ["detail", "migrate", "edit"].includes(view));

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand"><span className="logo">S</span>Session Bridge</div>
        {nav.map(([v, label]) => (
          <a className={`nav ${navOn(v) ? "on" : ""}`} key={v}
            onClick={() => setView(v)}>{label}</a>
        ))}
        <div className="spacer" />
        <TrustPanel env={env} />
      </aside>
      <main>
        {view === "library" &&
          <Library scan={scan} scanning={scanning} env={env}
            onScan={doScan} onOpen={openDetail} />}
        {view === "detail" && detail &&
          <Detail detail={detail} env={env}
            onBack={() => setView("library")}
            onMigrate={() => {
              setMig({ meta: detail.meta, ref: detail.ref, stage: "pick" });
              setView("migrate");
            }}
            onEdit={() => {
              setEdit({ meta: detail.meta, ref: detail.ref, data: detail.data,
                delTurns: new Set(), truncate: false, threshold: 4096,
                rewrite: null, preview: null, applying: false, result: null });
              setView("edit");
            }} />}
        {view === "migrate" && mig &&
          <Migrate mig={mig} setMig={setMig} env={env}
            onBack={() => setView("library")}
            onBackDetail={() => setView("detail")}
            gotoHistory={() => setView("history")} />}
        {view === "edit" && edit &&
          <Edit edit={edit} setEdit={setEdit}
            onBack={() => { setView("library"); doScan(); }}
            onBackDetail={() => setView("detail")}
            gotoSnapshots={() => setView("snapshots")}
            afterApply={() => {}} />}
        {view === "history" && <History />}
        {view === "snapshots" && <Snapshots />}
      </main>
    </div>
  );
}
