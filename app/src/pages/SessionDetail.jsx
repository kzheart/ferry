// 会话详情:头部 + 会话树 chips + 按轮时间线;编辑模式附 Inspector
import { useMemo, useState } from "react";
import { ACCENT, TOOL_NAME, fmtSize, fmtTime, resumeCommand, toRounds } from "../api.js";
import { Caret, Spinner, ToolIcon } from "../icons.jsx";
import { RadioDot } from "../components/ui.jsx";

const BIG_OUT = 4096;   // 与后端 truncate 默认阈值一致

function ToolCard({ t, k, open, trimmed, onToggle, onTrim, trimStaged }) {
  const big = (t.size || 0) > BIG_OUT;
  const cmd = typeof t.input === "object"
    ? (t.input.command || t.input.file_path || t.input.pattern ||
       JSON.stringify(t.input).slice(0, 80))
    : String(t.input || "").slice(0, 80);
  const hideBody = trimmed && !open;
  return (
    <div style={{ margin: "6px 0 6px 31px", border: "1px solid var(--line3)", borderRadius: 8,
      overflow: "hidden", background: "var(--fill)" }}>
      <div onClick={onToggle} style={{ display: "flex", alignItems: "center", gap: 9,
        padding: "7px 11px", cursor: "pointer" }}>
        <Caret open={open} size={10} />
        <span className="mono" style={{ fontSize: 11.5, fontWeight: 600, color: "var(--tx2b)" }}>{t.name}</span>
        <span className="mono" style={{ fontSize: 11.5, color: "var(--tx4)", whiteSpace: "nowrap",
          overflow: "hidden", textOverflow: "ellipsis", flex: 1 }}>{cmd}</span>
        {big && <span style={{ fontSize: 10.5, color: "var(--warn-deep)", background: "var(--warn-bg)",
          padding: "1px 7px", borderRadius: 20, flex: "none" }}>大输出 {fmtSize(t.size)}</span>}
      </div>
      {open && (
        <div style={{ borderTop: "1px solid var(--line5)" }}>
          {trimStaged && (
            <div style={{ padding: "10px 12px", fontSize: 12, color: "var(--tx4)",
              display: "flex", alignItems: "center", gap: 10 }}>
              <span>输出将被裁剪 · 保留前 {BIG_OUT} 字符 · 原始 {fmtSize(t.size)}</span>
            </div>
          )}
          {!hideBody && (
            <pre className="mono fscroll selectable" style={{ margin: 0, padding: "11px 13px",
              fontSize: 11.5, lineHeight: 1.6, color: "var(--tx2b)", whiteSpace: "pre-wrap",
              maxHeight: 200, overflow: "auto", background: "var(--surface)" }}>
              {(t.output || "(无输出)").slice(0, 200000)}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function Round({ r, editable, staged, onDelete, onTrim, onRewrite, migratable,
  scopeOn, onScope, onClearScope, onMigrateScope, scopeStats }) {
  const [open, setOpen] = useState({});
  const hasBigOut = r.tools.some(t => (t.size || 0) > BIG_OUT);
  return (
    <div style={{ marginBottom: 6 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 9, margin: "14px 0 8px" }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--tx5)", letterSpacing: ".03em" }}>第 {r.n} 轮</span>
        <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
        {editable && (
          <div style={{ display: "flex", gap: 5 }}>
            <button onClick={onDelete} style={{ height: 22, padding: "0 8px", border: "1px solid var(--err-line)",
              background: "var(--surface)", color: "var(--err-deep)", borderRadius: 6, fontSize: 11, cursor: "pointer" }}>删除</button>
            {hasBigOut && <button onClick={onTrim} style={{ height: 22, padding: "0 8px",
              border: "1px solid var(--line2)", background: "var(--surface)", color: "var(--tx3)", borderRadius: 6,
              fontSize: 11, cursor: "pointer" }}>裁剪</button>}
            {r.uuid && <button onClick={onRewrite} style={{ height: 22, padding: "0 8px",
              border: "1px solid var(--line2)", background: "var(--surface)", color: "var(--tx3)", borderRadius: 6,
              fontSize: 11, cursor: "pointer" }}>改写</button>}
          </div>
        )}
      </div>
      {staged && (
        <div style={{ margin: "0 0 8px", padding: "6px 10px", borderRadius: 7, background: "var(--warn-bg)",
          border: "1px solid var(--warn-line)", fontSize: 11.5, color: "var(--warn-text)" }}>已暂存:{staged}</div>
      )}
      {r.user && (
        <div style={{ display: "flex", justifyContent: "flex-end", margin: "6px 0" }}>
          <div className="selectable" style={{ maxWidth: "82%", background: ACCENT, color: "#fff",
            padding: "9px 13px", borderRadius: "12px 12px 3px 12px", fontSize: 13,
            whiteSpace: "pre-wrap", overflowWrap: "break-word" }}>
            {r.user.slice(0, 4000)}</div>
        </div>
      )}
      {r.ai.length > 0 && (
        <div style={{ display: "flex", gap: 9, margin: "6px 0" }}>
          <span style={{ width: 22, height: 22, flex: "none", borderRadius: 6, background: "var(--chip)",
            border: "1px solid var(--line)", display: "inline-flex", alignItems: "center",
            justifyContent: "center", fontSize: 10, color: "var(--tx3b)", fontWeight: 700 }}>AI</span>
          <div className="selectable" style={{ flex: 1, background: "var(--surface)", border: "1px solid var(--line5)",
            padding: "9px 13px", borderRadius: "3px 12px 12px 12px", fontSize: 13, color: "var(--tx1b)",
            whiteSpace: "pre-wrap", overflowWrap: "break-word" }}>
            {r.ai.join("\n\n").slice(0, 8000)}</div>
        </div>
      )}
      {r.tools.map((t, i) => (
        <ToolCard key={i} t={t} open={open[i] ?? false}
          onToggle={() => setOpen(o => ({ ...o, [i]: !(o[i] ?? false) }))}
          trimStaged={staged && staged.includes("裁剪")} />
      ))}
      {migratable && (
        <div style={{ margin: "8px 0 4px 31px" }}>
          {!scopeOn ? (
            <button data-guide={r.n === 1 ? "scope" : undefined} onClick={onScope}
              style={{ height: 26, padding: "0 11px", background: "var(--surface)",
                border: "1px dashed var(--acc-line2)", color: ACCENT, borderRadius: 7, fontSize: 12,
                cursor: "pointer", fontWeight: 500 }}>↧ 迁移到此为止</button>
          ) : (
            <div style={{ border: "1px solid var(--acc-line2)", background: "var(--acc-soft5)", borderRadius: 9,
              padding: "11px 13px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontWeight: 600, color: "var(--acc-text)", fontSize: 12.5 }}>仅迁移到第 {r.n} 轮</span>
                <a onClick={onClearScope} style={{ fontSize: 11.5, marginLeft: "auto", color: "var(--tx3b)" }}>取消</a>
              </div>
              <div style={{ fontSize: 12, color: "var(--tx2b)", marginTop: 5 }}>{scopeStats}</div>
              <button className="fbtn-primary" onClick={onMigrateScope}
                style={{ marginTop: 9, height: 28, padding: "0 13px", fontSize: 12 }}>用此范围开始迁移</button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Inspector({ ops, removeOp, updateOp, saveMode, setSaveMode, sizeInfo,
  onOpenDiff, onApply, applying, canEdit }) {
  const hasOps = ops.length > 0;
  const modes = [
    ["saveas", "另存为新会话", "保留原会话不变(默认)"],
    ["inplace", "原地修改", "改写原始会话文件 · 需二次确认"],
  ];
  return (
    <div style={{ width: 300, flex: "none", borderLeft: "1px solid var(--line)", background: "var(--inset)",
      display: "flex", flexDirection: "column", minHeight: 0 }}>
      <div style={{ padding: "14px 16px 12px", borderBottom: "1px solid var(--line5)" }}>
        <div style={{ fontSize: 13, fontWeight: 650 }}>编辑 Inspector</div>
        <div style={{ fontSize: 11.5, color: "var(--tx4)", marginTop: 3 }}>暂存的操作在应用前不会改动原会话</div>
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "12px 14px" }}>
        {!hasOps && (
          <div style={{ padding: "26px 12px", textAlign: "center", color: "var(--tx5)", fontSize: 12,
            border: "1px dashed var(--line2)", borderRadius: 9 }}>
            在左侧时间线对某一轮点击<br />删除 / 裁剪 / 改写以暂存操作</div>
        )}
        {ops.map(o => (
          <div key={o.id} style={{ padding: "9px 10px", border: "1px solid var(--line3)", borderRadius: 8,
            background: "var(--surface)", marginBottom: 7 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
              <span style={{ width: 6, height: 6, borderRadius: "50%", background: o.dot, flex: "none" }} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 12.5, color: "var(--tx2)", fontWeight: 500 }}>{o.label}</div>
                <div style={{ fontSize: 11, color: "var(--tx5)" }}>{o.delta}</div>
              </div>
              <a onClick={() => removeOp(o.id)} style={{ color: "var(--tx5)", fontSize: 14 }}>×</a>
            </div>
            {o.type === "rewrite" && (
              <textarea className="fscroll" value={o.text}
                onChange={e => updateOp(o.id, { text: e.target.value })}
                style={{ width: "100%", marginTop: 8, minHeight: 64, resize: "vertical",
                  border: "1px solid var(--line)", borderRadius: 6, padding: "6px 8px",
                  fontSize: 12, color: "var(--tx2)", outline: "none", userSelect: "text" }} />
            )}
          </div>
        ))}
      </div>
      <div style={{ flex: "none", borderTop: "1px solid var(--line5)", padding: "13px 16px" }}>
        <div style={{ display: "flex", justifyContent: "space-between", fontSize: 12,
          color: "var(--tx3b)", marginBottom: 4 }}>
          <span>体积变化</span>
          <span style={{ color: "var(--ok-deep)", fontWeight: 600 }}>{sizeInfo.delta}</span>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, color: "var(--tx4)" }}>
          <span>{sizeInfo.before}</span><span>→</span>
          <span style={{ color: "var(--tx2)", fontWeight: 600 }}>{sizeInfo.after}</span>
        </div>
        <div style={{ height: 6, borderRadius: 4, background: "var(--track)", marginTop: 8, overflow: "hidden" }}>
          <div style={{ height: "100%", width: sizeInfo.barW, background: "var(--ok-deep)",
            transition: "width .3s ease" }} />
        </div>
        <div style={{ marginTop: 14, fontSize: 11.5, fontWeight: 600, color: "var(--tx3b)" }}>保存方式</div>
        {modes.map(([k, l, d]) => {
          const on = saveMode === k;
          return (
            <label key={k} onClick={() => setSaveMode(k)}
              style={{ display: "flex", alignItems: "flex-start", gap: 9, padding: "8px 9px",
                border: `1px solid ${on ? ACCENT : "var(--line3)"}`, background: on ? "var(--acc-soft4)" : "var(--surface)",
                borderRadius: 8, marginTop: 7, cursor: "pointer" }}>
              <span style={{ marginTop: 1, display: "inline-flex" }}><RadioDot on={on} /></span>
              <span>
                <span style={{ fontSize: 12.5, color: "var(--tx2)", fontWeight: 500 }}>{l}</span><br />
                <span style={{ fontSize: 11, color: "var(--tx5)" }}>{d}</span>
              </span>
            </label>
          );
        })}
        <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 10, lineHeight: 1.5 }}>
          应用前自动创建快照;探针失败将自动还原到应用前状态。</div>
        <div style={{ display: "flex", gap: 8, marginTop: 11 }}>
          <button className="fbtn" style={{ flex: 1, height: 32, fontSize: 12.5 }}
            onClick={onOpenDiff}>预览差异</button>
          <button className="fbtn-primary" style={{ flex: 1, height: 32 }}
            disabled={!hasOps || applying || !canEdit} onClick={onApply}>
            {applying ? "应用中…" : hasOps ? "应用更改" : "无待应用"}</button>
        </div>
        {!canEdit && <div style={{ fontSize: 11, color: "var(--warn-deep)", marginTop: 8 }}>
          目前仅支持编辑 Claude Code 会话</div>}
      </div>
    </div>
  );
}

export default function SessionDetail({ meta, data, error, mode, onEnterEdit, onExitEdit,
  scope, setScope, ops, addOp, removeOp, updateOp, saveMode, setSaveMode,
  onOpenDiff, onApply, applying, onOpenMigrate }) {
  const rounds = useMemo(() => toRounds(data?.messages), [data]);
  const isEdit = mode === "edit";
  const canEdit = meta.tool === "claude";
  const [copied, setCopied] = useState(false);

  const roundSize = r => (r.user?.length || 0) + r.ai.join("").length +
    r.tools.reduce((a, t) => a + (t.size || 0), 0);
  const totalSize = meta.size || rounds.reduce((a, r) => a + roundSize(r), 0);
  const delta = ops.reduce((a, o) => a + (o.bytes || 0), 0);
  const after = Math.max(0, totalSize - delta);
  const sizeInfo = {
    before: fmtSize(totalSize), after: fmtSize(after),
    delta: delta > 0 ? `−${fmtSize(delta)}` : "0 B",
    barW: totalSize ? `${Math.round(after * 100 / totalSize)}%` : "100%",
  };

  const resume = resumeCommand(meta.tool, meta.id, meta.dir);
  const copyResume = () => {
    try { navigator.clipboard?.writeText(resume); } catch {}
    setCopied(true); setTimeout(() => setCopied(false), 1600);
  };

  const scopeMsgs = scope
    ? (rounds.slice(0, scope).reduce((a, r) => a + 1 + (r.ai.length ? 1 : 0), 0) +
       rounds.slice(0, scope).reduce((a, r) => a + r.tools.length, 0))
    : 0;
  const scopeStats = scope
    ? `约 ${scopeMsgs} 条消息 · ${fmtSize(rounds.slice(0, scope).reduce((a, r) => a + roundSize(r), 0))}`
    : "";

  const stagedFor = n => {
    const o = ops.find(o => o.n === n);
    return o ? o.label : null;
  };

  const treeChips = [
    { label: `${TOOL_NAME[meta.tool]} 会话`, bg: "var(--chip)" },
    ...(data && data.tree_count > 1
      ? [{ label: `${data.tree_count - 1} 个子会话` }] : []),
    { label: `${data ? data.count : meta.count} 条消息` },
  ];

  return (
    <div style={{ flex: 1, display: "flex", minWidth: 0, minHeight: 0 }}>
      <div className="fscroll" data-guide-scroll="1"
        style={{ flex: 1, overflowY: "auto", minWidth: 0, animation: "ffade .16s ease" }}>
        <div style={{ padding: "18px 22px 14px", borderBottom: "1px solid var(--line5)", position: "sticky",
          top: 0, background: "var(--veil)", backdropFilter: "blur(6px)", zIndex: 2 }}>
          <div style={{ display: "flex", alignItems: "flex-start", gap: 13 }}>
            <ToolIcon tool={meta.tool} size={40} dot="var(--ok)" />
            <div style={{ flex: 1, minWidth: 0 }}>
              <div style={{ fontSize: 16, fontWeight: 650, letterSpacing: "-.01em" }}>
                {meta.title || "(无标题会话)"}</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: "6px 14px", marginTop: 6,
                fontSize: 12, color: "var(--tx3b)" }}>
                <span>来源 <b style={{ color: "var(--tx2)", fontWeight: 600 }}>{TOOL_NAME[meta.tool]}</b></span>
                <span className="mono" style={{ color: "var(--tx4)" }}>{meta.dir}</span>
                <span>{data ? data.count : meta.count} 条消息</span>
                <span>{fmtSize(meta.size)}</span>
                <span>活跃 {fmtTime(meta.updated)}</span>
              </div>
            </div>
            {!isEdit ? (
              <div style={{ display: "flex", gap: 8, flex: "none" }}>
                <button className="fbtn" style={{ height: 30, fontSize: 12.5 }} onClick={copyResume}>
                  {copied ? "已复制接续命令" : "复制接续命令"}</button>
                <button className="fbtn" style={{ height: 30, fontSize: 12.5 }}
                  onClick={onEnterEdit} disabled={!data}>会话编辑</button>
                <button data-guide="migrate" className="fbtn-primary"
                  style={{ height: 30, padding: "0 14px" }}
                  onClick={() => onOpenMigrate(null)}>迁移…</button>
              </div>
            ) : (
              <div style={{ display: "flex", alignItems: "center", gap: 10, flex: "none" }}>
                <span style={{ fontSize: 12, color: "var(--warn-deep)", background: "var(--warn-bg)",
                  border: "1px solid var(--warn-line)", padding: "3px 9px", borderRadius: 20,
                  fontWeight: 600 }}>编辑模式</span>
                <button className="fbtn" style={{ height: 30, fontSize: 12.5 }}
                  onClick={onExitEdit}>退出编辑</button>
              </div>
            )}
          </div>
          <div className="mono" style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 13,
            fontSize: 11.5, color: "var(--tx4)" }}>
            {treeChips.map((t, i) => (
              <span key={i} style={{ display: "inline-flex", alignItems: "center", gap: 7 }}>
                <span style={{ padding: "2px 8px", borderRadius: 6, background: t.bg || "transparent",
                  color: "var(--tx3b)" }}>{t.label}</span>
                {i < treeChips.length - 1 && <span>→</span>}
              </span>
            ))}
          </div>
        </div>
        <div style={{ padding: "16px 22px 40px", maxWidth: 760 }}>
          {error && <div style={{ padding: 30, color: "var(--err-deep)", fontSize: 13 }}>读取失败:{error}</div>}
          {!data && !error && (
            <div style={{ padding: 40, display: "flex", alignItems: "center", gap: 10,
              color: "var(--tx4)", fontSize: 13 }}><Spinner size={16} /> 解析会话中…</div>
          )}
          {data && rounds.map(r => (
            <Round key={r.n} r={r} editable={isEdit && canEdit}
              staged={stagedFor(r.n)}
              onDelete={() => addOp("delete", r)}
              onTrim={() => addOp("trim", r)}
              onRewrite={() => addOp("rewrite", r)}
              migratable={!isEdit && r.n < rounds.length}
              scopeOn={scope === r.n}
              onScope={() => setScope(r.n)}
              onClearScope={() => setScope(null)}
              onMigrateScope={() => onOpenMigrate(r.n)}
              scopeStats={scopeStats} />
          ))}
          {data && rounds.length === 0 && (
            <div style={{ padding: 30, color: "var(--tx5)", fontSize: 12.5 }}>该会话没有可展示的消息</div>
          )}
        </div>
      </div>
      {isEdit && (
        <Inspector ops={ops} removeOp={removeOp} updateOp={updateOp}
          saveMode={saveMode} setSaveMode={setSaveMode} sizeInfo={sizeInfo}
          onOpenDiff={onOpenDiff} onApply={onApply} applying={applying} canEdit={canEdit} />
      )}
    </div>
  );
}
