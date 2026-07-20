// 会话详情:头部 + 会话树 chips + 按轮时间线;编辑模式附 Inspector
import { useMemo, useRef, useState } from "react";
import { ACCENT, TOOL_NAME, fmtSize, fmtTime, resumeCommand, toRounds } from "../api.js";
import { BookmarkIcon, Caret, CheckIcon, CloseIcon, CopyIcon, ImageGlyph,
  PencilIcon, ScissorsIcon, Spinner, ToolIcon, TrashIcon, UndoIcon } from "../icons.jsx";
import { RadioDot } from "../components/ui.jsx";
import Markdown from "../components/Markdown.jsx";

const BIG_OUT = 4096;   // 与后端 truncate 默认阈值一致

// 用户消息里的图片占位(粘贴图片的缓存路径)不直出,收成计数
const IMG_RE = /\[Image:\s*source:[^\]]*\]|\[Image #\d+\]/g;
const splitImages = text => {
  const imgs = (String(text || "").match(IMG_RE) || []).length;
  return { text: String(text || "").replace(IMG_RE, "").replace(/\n{3,}/g, "\n\n").trim(), imgs };
};

function IconBtn({ title, danger, accent, onClick, style, children, ...rest }) {
  return (
    <button title={title} onClick={onClick} {...rest}
      className={`ficon-btn${danger ? " danger" : ""}${accent ? " accent" : ""}`}
      style={style}>{children}</button>
  );
}

function ToolCard({ t, open, onToggle, trimStaged }) {
  const big = (t.size || 0) > BIG_OUT;
  const cmd = typeof t.input === "object"
    ? (t.input.command || t.input.file_path || t.input.pattern ||
       JSON.stringify(t.input).slice(0, 80))
    : String(t.input || "").slice(0, 80);
  const out = t.output || "(无输出)";
  const cut = trimStaged && big;   // 已暂存裁剪:超阈值部分变淡+划线,所见即所裁
  return (
    <div style={{ margin: "5px 0", border: "1px solid var(--line3)", borderRadius: 8,
      overflow: "hidden", background: "var(--fill)" }}>
      <div onClick={onToggle} style={{ display: "flex", alignItems: "center", gap: 9,
        padding: "7px 11px", cursor: "pointer" }}>
        <Caret open={open} size={10} />
        <span className="mono" style={{ fontSize: 11.5, fontWeight: 600, color: "var(--tx2b)" }}>{t.name}</span>
        <span className="mono" style={{ fontSize: 11.5, color: "var(--tx4)", whiteSpace: "nowrap",
          overflow: "hidden", textOverflow: "ellipsis", flex: 1 }}>{cmd}</span>
        {cut ? (
          <span title={`裁剪后保留前 ${BIG_OUT} 字符 · 原始 ${fmtSize(t.size)}`}
            style={{ display: "inline-flex", alignItems: "center", gap: 5, fontSize: 10.5,
              color: "var(--warn-deep)", background: "var(--warn-bg)", padding: "1px 7px",
              borderRadius: 20, flex: "none" }}>
            <ScissorsIcon size={10} /> {fmtSize(t.size)} → {fmtSize(BIG_OUT)}</span>
        ) : big && (
          <span style={{ fontSize: 10.5, color: "var(--warn-deep)", background: "var(--warn-bg)",
            padding: "1px 7px", borderRadius: 20, flex: "none" }}>大输出 {fmtSize(t.size)}</span>
        )}
      </div>
      {open && (
        <pre className="mono fscroll selectable" style={{ margin: 0, padding: "11px 13px",
          fontSize: 11.5, lineHeight: 1.6, color: "var(--tx2b)", whiteSpace: "pre-wrap",
          maxHeight: 200, overflow: "auto", background: "var(--surface)",
          borderTop: "1px solid var(--line5)" }}>
          {cut ? (<>
            {out.slice(0, BIG_OUT)}
            <span className="ftrim-cut">{out.slice(BIG_OUT, 200000)}</span>
          </>) : out.slice(0, 200000)}
        </pre>
      )}
    </div>
  );
}

function Round({ r, editable, delOp, trimOn, rewOp, onDelete, onUndoDelete, onTrim,
  onRewrite, onUpdateRewrite, onCancelRewrite, migratable,
  scopeOn, onScope, onClearScope, onMigrateScope, scopeStats }) {
  const [open, setOpen] = useState({});
  const [toolsOpen, setToolsOpen] = useState(false);
  const [rewEditing, setRewEditing] = useState(false);
  const [copied, setCopied] = useState(false);
  const taRef = useRef(null);
  const hasBigOut = r.tools.some(t => (t.size || 0) > BIG_OUT);
  const { text: userText, imgs } = useMemo(() => splitImages(r.user), [r.user]);
  const aiText = (r.final || "").slice(0, 8000);
  const deleted = !!delOp;

  const copyAi = () => {
    try { navigator.clipboard?.writeText(aiText); } catch {}
    setCopied(true); setTimeout(() => setCopied(false), 1400);
  };
  const startRewrite = () => {
    onRewrite();
    setRewEditing(true);
    setTimeout(() => taRef.current?.focus(), 0);
  };

  return (
    <div className="fround" style={{ marginBottom: editable ? 10 : 30 }}>
      {editable && (
        <div style={{ display: "flex", alignItems: "center", gap: 9, margin: "10px 0 8px" }}>
          <span style={{ width: 20, height: 20, flex: "none", borderRadius: "50%",
            border: "1.5px solid var(--line2)", display: "inline-flex", alignItems: "center",
            justifyContent: "center", fontSize: 10.5, fontWeight: 700, color: "var(--tx4b)" }}>{r.n}</span>
          <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
          <div style={{ display: "flex", gap: 3 }}>
            {deleted ? (
              <IconBtn title="撤销删除" onClick={onUndoDelete}><UndoIcon /></IconBtn>
            ) : (
              <IconBtn title={`删除第 ${r.n} 轮`} danger onClick={onDelete}><TrashIcon /></IconBtn>
            )}
            {hasBigOut && !trimOn && !deleted &&
              <IconBtn title="裁剪超长工具输出" onClick={onTrim}><ScissorsIcon /></IconBtn>}
            {r.uuid && !deleted &&
              <IconBtn title="改写用户消息" onClick={startRewrite}><PencilIcon /></IconBtn>}
          </div>
        </div>
      )}
      <div className={deleted ? "fdel" : undefined}>
        {r.user && imgs > 0 && (
          <div style={{ display: "flex", justifyContent: "flex-end", margin: "6px 0 4px" }}>
            <span title={`${imgs} 张图片`} style={{ display: "inline-flex", alignItems: "center",
              gap: 5, padding: "2px 9px", borderRadius: 20, background: "var(--chip)",
              color: "var(--tx4)", fontSize: 10.5 }}>
              <ImageGlyph /> ×{imgs}</span>
          </div>
        )}
        {r.user && (
          <div style={{ display: "flex", justifyContent: "flex-end", margin: "6px 0" }}>
            {rewOp && rewEditing && !deleted ? (
              <div style={{ width: "82%" }}>
                <textarea ref={taRef} className="fscroll selectable" value={rewOp.text}
                  onChange={e => onUpdateRewrite(e.target.value)}
                  style={{ width: "100%", minHeight: 72, resize: "vertical", boxSizing: "border-box",
                    background: "var(--fill4)", color: "var(--tx1b)", border: `1.5px solid ${ACCENT}`,
                    padding: "10px 14px", borderRadius: 16, fontSize: 13, lineHeight: 1.65,
                    outline: "none", userSelect: "text", fontFamily: "inherit" }} />
                <div style={{ display: "flex", justifyContent: "flex-end", gap: 3, marginTop: 2 }}>
                  <IconBtn title="取消改写" onClick={() => { onCancelRewrite(); setRewEditing(false); }}>
                    <CloseIcon /></IconBtn>
                  <IconBtn title="确认改写" accent onClick={() => setRewEditing(false)}>
                    <CheckIcon /></IconBtn>
                </div>
              </div>
            ) : (
              <div className="fdel-text selectable" onClick={rewOp && !deleted ? startRewrite : undefined}
                title={rewOp && !deleted ? "点击继续编辑" : undefined}
                style={{ maxWidth: "82%", background: "var(--fill4)", color: "var(--tx1b)",
                  padding: "9px 14px", borderRadius: 16, fontSize: 13, lineHeight: 1.65,
                  whiteSpace: "pre-wrap", overflowWrap: "break-word",
                  cursor: rewOp && !deleted ? "text" : undefined }}>
                {(rewOp ? rewOp.text : userText).slice(0, 4000)}
                {rewOp && !deleted && (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 4, marginLeft: 8,
                    color: ACCENT, fontSize: 10.5, fontWeight: 600, whiteSpace: "nowrap" }}>
                    <PencilIcon size={10} /> 已改写</span>
                )}
              </div>
            )}
          </div>
        )}
        {r.steps.length > 0 && (
          <div style={{ margin: "8px 0" }}>
            <div onClick={() => setToolsOpen(v => !v)} style={{ display: "inline-flex",
              alignItems: "center", gap: 6, padding: "3px 8px 3px 4px", borderRadius: 7,
              cursor: "pointer", color: "var(--tx4)", fontSize: 12 }} className="hov-ghost">
              <Caret open={toolsOpen} size={9} />
              <span>{r.steps.length} 步</span>
            </div>
            {toolsOpen && (
              <div style={{ marginLeft: 18, marginTop: 2, borderLeft: "2px solid var(--line5)",
                paddingLeft: 13 }}>
                {r.steps.map((s, i) => s.kind === "text" ? (
                  <div key={i} className="selectable" style={{ margin: "7px 0", fontSize: 12.5,
                    lineHeight: 1.65, color: "var(--tx3b)", whiteSpace: "pre-wrap",
                    overflowWrap: "break-word" }}>{s.text.slice(0, 4000)}</div>
                ) : (
                  <ToolCard key={i} t={s.tool} open={open[i] ?? false}
                    onToggle={() => setOpen(o => ({ ...o, [i]: !(o[i] ?? false) }))}
                    trimStaged={trimOn} />
                ))}
              </div>
            )}
          </div>
        )}
        {aiText && (
          <div style={{ margin: "10px 0 0" }}>
            <div className="fdel-text"><Markdown text={aiText} /></div>
            {!editable && (
              <div className="fhact" style={{ marginTop: 4 }}>
                <IconBtn title={copied ? "已复制" : "复制回复"} onClick={copyAi}>
                  {copied ? <CheckIcon /> : <CopyIcon />}</IconBtn>
              </div>
            )}
          </div>
        )}
      </div>
      {migratable && (
        <div style={{ margin: "10px 0 0" }}>
          {!scopeOn ? (
            <div className={r.n === 1 ? undefined : "fhact"}
              style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
              <button data-guide={r.n === 1 ? "scope" : undefined}
                className="ficon-btn accent" onClick={onScope}
                style={{ width: "auto", padding: "0 11px", gap: 6, fontSize: 11.5, fontWeight: 500,
                  border: "1px dashed var(--acc-line2)", borderRadius: 13 }}>
                <BookmarkIcon /> 迁移到此为止</button>
              <span style={{ flex: 1, height: 1, background: "var(--hairline)" }} />
            </div>
          ) : (
            <div style={{ border: "1px solid var(--acc-line2)", background: "var(--acc-soft5)", borderRadius: 9,
              padding: "11px 13px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontWeight: 600, color: "var(--acc-text)", fontSize: 12.5 }}>仅迁移到第 {r.n} 轮</span>
                <IconBtn title="取消" onClick={onClearScope} style={{ marginLeft: "auto" }}><CloseIcon /></IconBtn>
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
              <IconBtn title="撤销此操作" onClick={() => removeOp(o.id)}><CloseIcon size={11} /></IconBtn>
            </div>
            {o.type === "rewrite" && (
              <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 6 }}>
                在左侧时间线的气泡里直接编辑内容</div>
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
          应用前自动创建快照;验收未通过将自动还原到应用前状态。</div>
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

  const opFor = (n, type) => ops.find(o => o.type === type && o.n === n);
  const trimOn = ops.some(o => o.type === "trim");

  const subCount = data ? data.tree_count - 1 : 0;

  return (
    <div style={{ flex: 1, display: "flex", minWidth: 0, minHeight: 0 }}>
      <div className="fscroll" data-guide-scroll="1"
        style={{ flex: 1, overflowY: "auto", minWidth: 0, animation: "ffade .16s ease" }}>
        <div style={{ padding: "18px 26px 14px", borderBottom: "1px solid var(--line5)", position: "sticky",
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
          {subCount > 0 && (
            <div className="mono" style={{ display: "flex", alignItems: "center", gap: 7, marginTop: 13,
              fontSize: 11.5, color: "var(--tx4)" }}>
              <span style={{ padding: "2px 8px", borderRadius: 6, background: "var(--chip)",
                color: "var(--tx3b)" }}>{TOOL_NAME[meta.tool]} 会话</span>
              <span>→</span>
              <span style={{ padding: "2px 8px", borderRadius: 6, color: "var(--tx3b)" }}>
                {subCount} 个子会话</span>
            </div>
          )}
        </div>
        <div style={{ padding: "20px 26px 48px", maxWidth: 720, margin: "0 auto" }}>
          {error && <div style={{ padding: 30, color: "var(--err-deep)", fontSize: 13 }}>读取失败:{error}</div>}
          {!data && !error && (
            <div style={{ padding: 40, display: "flex", alignItems: "center", gap: 10,
              color: "var(--tx4)", fontSize: 13 }}><Spinner size={16} /> 解析会话中…</div>
          )}
          {data && rounds.map(r => (
            <Round key={r.n} r={r} editable={isEdit && canEdit}
              delOp={opFor(r.n, "delete")} trimOn={trimOn} rewOp={opFor(r.n, "rewrite")}
              onDelete={() => addOp("delete", r)}
              onUndoDelete={() => { const o = opFor(r.n, "delete"); if (o) removeOp(o.id); }}
              onTrim={() => addOp("trim", r)}
              onRewrite={() => addOp("rewrite", r)}
              onUpdateRewrite={text => { const o = opFor(r.n, "rewrite"); if (o) updateOp(o.id, { text }); }}
              onCancelRewrite={() => { const o = opFor(r.n, "rewrite"); if (o) removeOp(o.id); }}
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
