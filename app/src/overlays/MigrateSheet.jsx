// 迁移向导:目标 → 预演(dry_run) → 确认 → 写入 → 结果(成功/失败+摘要兜底)
import { useEffect, useRef, useState } from "react";
import { ACCENT, TOOL_NAME, TOOLS, fmtSize, openTerminal, rpc, sessionRef } from "../api.js";
import { CheckBadge, Spinner, ToolIcon, WarnTriangle } from "../icons.jsx";
import { CheckSquare, CmdRow, LossCols, Sheet } from "../components/ui.jsx";

const ORDER = ["target", "preview", "confirm", "result"];

function StepsHeader({ step }) {
  const labels = { target: "目标", preview: "预演", confirm: "确认",
    result: step === "writing" ? "写入" : "结果" };
  const cur = ORDER.indexOf(step === "writing" ? "result" : step);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 7, marginLeft: 6 }}>
      {ORDER.map((s, i) => (
        <span key={s} style={{ display: "inline-flex", alignItems: "center", gap: 7 }}>
          <span style={{ fontSize: 11.5, fontWeight: 600,
            color: i === cur ? ACCENT : i < cur ? "#5B6672" : "#C3CBD3" }}>{labels[s]}</span>
          {i < ORDER.length - 1 && <span style={{ color: "#C3CBD3", fontSize: 11 }}>›</span>}
        </span>
      ))}
    </div>
  );
}

export default function MigrateSheet({ meta, scope, env, onClose, onDone }) {
  const targets = TOOLS.filter(t => t !== meta.tool);
  const [step, setStep] = useState("target");
  const [target, setTarget] = useState(targets[0]);
  const [dry, setDry] = useState(null);        // { [target]: result }
  const [dryErr, setDryErr] = useState(null);
  const [redact, setRedact] = useState(true);
  const [result, setResult] = useState(null);
  const [error, setError] = useState(null);
  const [wroteFirst, setWroteFirst] = useState(false);
  const [handoff, setHandoff] = useState(null);
  const [handoffBusy, setHandoffBusy] = useState(false);
  const doneRef = useRef(false);
  const ref = sessionRef(meta);

  const d = dry?.[target];
  const sensitive = d?.sensitive;
  const scopeLabel = scope ? `仅迁移到第 ${scope} 轮` : "完整会话";

  const loadDry = async tgt => {
    setDryErr(null);
    try {
      const r = await rpc("migrate", { src: meta.tool, dst: tgt, ref,
        dry_run: true, max_turn: scope || undefined });
      setDry(prev => ({ ...prev, [tgt]: r }));
    } catch (e) { setDryErr(e.message); }
  };

  const next = () => {
    if (step === "target") { if (!dry?.[target]) loadDry(target); setStep("preview"); }
    else if (step === "preview") setStep("confirm");
    else if (step === "confirm") execute();
  };
  const back = () => {
    if (step === "preview") setStep("target");
    else if (step === "confirm") setStep("preview");
  };

  const execute = async () => {
    setStep("writing");
    setWroteFirst(false);
    setTimeout(() => setWroteFirst(true), 1500);
    try {
      const r = await rpc("migrate", { src: meta.tool, dst: target, ref,
        redact: redact && (sensitive?.total || 0) > 0,
        max_turn: scope || undefined });
      setResult(r);
    } catch (e) { setError(e.message); }
    setStep("result");
    if (!doneRef.current) { doneRef.current = true; onDone?.(); }
  };

  const doHandoff = async () => {
    if (handoff) return;
    setHandoffBusy(true);
    try {
      setHandoff(await rpc("handoff", { src: meta.tool, dst: target, ref }));
    } catch (e) { setError(e.message); }
    setHandoffBusy(false);
  };

  const ok = result && result.probe?.ok !== false && result.session_id;
  const fail = step === "result" && !ok;
  const installed = t => env?.[t]?.installed;

  let body = null;
  if (step === "target") {
    body = (
      <>
        <div style={{ fontSize: 13, color: "#6B7682", marginBottom: 6 }}>
          源会话 <b style={{ color: "#334155" }}>{meta.title || meta.id}</b> · {scopeLabel}</div>
        <div style={{ fontSize: 12, color: "#8A939D", marginBottom: 14 }}>
          选择迁移目标工具(源会话保持只读,不会被修改)</div>
        {targets.map(t => {
          const on = target === t;
          const inst = installed(t);
          return (
            <div key={t} onClick={() => inst && setTarget(t)}
              style={{ display: "flex", alignItems: "center", gap: 12, padding: "13px 14px",
                border: `1.5px solid ${on ? ACCENT : "#E4E9EE"}`, background: on ? "#F1F6FE" : "#fff",
                borderRadius: 10, marginBottom: 9, cursor: inst ? "pointer" : "not-allowed",
                opacity: inst ? 1 : 0.55 }}>
              <ToolIcon tool={t} size={32} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13.5, fontWeight: 600, color: "#334155" }}>{TOOL_NAME[t]}</div>
                <div style={{ fontSize: 11.5, color: "#8A939D" }}>
                  {inst ? `v${env[t].version || "?"} · 写入 ${t} 的本地会话存储` : "未检测到安装,无法作为目标"}
                </div>
              </div>
              <span style={{ width: 18, height: 18, borderRadius: "50%",
                border: `2px solid ${on ? ACCENT : "#C3CBD3"}`, display: "inline-flex",
                alignItems: "center", justifyContent: "center" }}>
                <span style={{ width: 9, height: 9, borderRadius: "50%",
                  background: on ? ACCENT : "transparent" }} />
              </span>
            </div>
          );
        })}
      </>
    );
  } else if (step === "preview") {
    body = !d ? (
      <div style={{ padding: "60px 0", display: "flex", alignItems: "center", justifyContent: "center",
        gap: 10, color: "#8A939D", fontSize: 13 }}>
        {dryErr ? <span style={{ color: "#B4433A" }}>预演失败:{dryErr}</span>
          : <><Spinner size={16} /> 正在预演转换、计算损耗…</>}
      </div>
    ) : (
      <>
        <div style={{ display: "flex", alignItems: "center", gap: 9, marginBottom: 14 }}>
          <ToolIcon tool={meta.tool} size={24} />
          <span style={{ color: "#8A939D", fontSize: 12.5 }}>{TOOL_NAME[meta.tool]}</span>
          <span style={{ color: "#C3CBD3" }}>→</span>
          <ToolIcon tool={target} size={26} />
          <span style={{ fontSize: 13, fontWeight: 600, color: "#334155" }}>{TOOL_NAME[target]}</span>
          <span style={{ marginLeft: "auto", fontSize: 11.5, color: "#6B7682", background: "#EEF2F6",
            border: "1px solid #E1E7EC", padding: "3px 10px", borderRadius: 20 }}>只读源 · 不修改</span>
        </div>
        <div style={{ fontSize: 12, fontWeight: 600, color: "#6B7682", marginBottom: 8 }}>
          损耗预演 · {scopeLabel}</div>
        <div style={{ marginBottom: 16 }}><LossCols loss={d.loss} /></div>
        <div style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "13px 15px", marginBottom: 12,
          display: "flex", justifyContent: "space-between", alignItems: "center", fontSize: 12.5 }}>
          <span style={{ color: "#334155", fontWeight: 600 }}>迁移规模</span>
          <span className="mono" style={{ color: "#334155" }}>
            {d.msg_count} 条消息 · {d.tree_count} 个树节点</span>
        </div>
        {sensitive?.total > 0 ? (
          <div style={{ border: "1px solid #EBCBC7", background: "#FDF3F1", borderRadius: 10,
            padding: "12px 14px", display: "flex", gap: 11, alignItems: "flex-start" }}>
            <WarnTriangle />
            <div style={{ flex: 1 }}>
              <div style={{ fontSize: 12.5, fontWeight: 600, color: "#8A3E37" }}>检测到疑似敏感信息</div>
              <div style={{ fontSize: 11.5, color: "#96524B", marginTop: 3 }}>
                {sensitive.findings.map(f => `${f.count} 处${f.label}`).join("、")}。建议脱敏后再迁移。</div>
            </div>
            <label onClick={() => setRedact(v => !v)}
              style={{ display: "flex", alignItems: "center", gap: 7, cursor: "pointer", flex: "none" }}>
              <CheckSquare on={redact} accent="#C4564C" />
              <span style={{ fontSize: 11.5, color: "#8A3E37" }}>迁移前脱敏</span>
            </label>
          </div>
        ) : (
          <div style={{ border: "1px solid #CDE9D7", background: "#F1FBF5", borderRadius: 10,
            padding: "12px 14px", fontSize: 12, color: "#1C7C43" }}>未检测到敏感信息</div>
        )}
      </>
    );
  } else if (step === "confirm") {
    body = (
      <>
        <div style={{ border: "1px solid #E4E9EE", borderRadius: 10, padding: "16px 18px" }}>
          <div style={{ fontSize: 13.5, fontWeight: 650, marginBottom: 12 }}>确认迁移</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 9, fontSize: 12.5 }}>
            {[["目标", TOOL_NAME[target], true],
              ["范围", `${scopeLabel}${d ? ` · ${d.msg_count} 条` : ""}`],
              ["脱敏", sensitive?.total > 0
                ? (redact ? `迁移前脱敏 ${sensitive.findings.map(f => `${f.count} 处${f.label}`).join("、")}` : "不脱敏")
                : "无需(未检测到敏感信息)"],
            ].map(([k, v, bold], i) => (
              <div key={i} style={{ display: "flex", justifyContent: "space-between", gap: 20 }}>
                <span style={{ color: "#8A939D", flex: "none" }}>{k}</span>
                <span style={{ color: "#334155", fontWeight: bold ? 600 : 400, textAlign: "right" }}>{v}</span>
              </div>
            ))}
            <div style={{ display: "flex", justifyContent: "space-between" }}>
              <span style={{ color: "#8A939D" }}>源会话</span>
              <span style={{ color: "#1C7C43", fontWeight: 600 }}>只读 · 不修改</span>
            </div>
          </div>
        </div>
        <div style={{ fontSize: 12, color: "#6B7682", margin: "14px 0 0", lineHeight: 1.55 }}>
          Ferry 将写入目标工具,然后运行探针验收(校验消息完整性与可续接性,需数十秒并消耗一次极小的模型调用)。若探针失败,会自动回滚,不在目标保留任何产物。</div>
      </>
    );
  } else if (step === "writing") {
    const items = [
      { label: `写入 ${TOOL_NAME[target]} 会话存储`, state: wroteFirst ? "done" : "spin" },
      { label: "探针验收(完整性 · 可续接性)", state: wroteFirst ? "spin" : "wait" },
    ];
    body = (
      <div style={{ padding: "24px 6px" }}>
        {items.map((p, i) => (
          <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 0" }}>
            {p.state === "spin" && <Spinner size={18} track="#E1E7EC" />}
            {p.state === "done" && <CheckBadge />}
            {p.state === "wait" && <span style={{ width: 18, height: 18, borderRadius: "50%",
              border: "2px solid #E1E7EC", flex: "none" }} />}
            <span style={{ fontSize: 13, color: p.state === "spin" ? ACCENT : "#334155",
              fontWeight: p.state === "spin" ? 600 : 500 }}>{p.label}</span>
          </div>
        ))}
      </div>
    );
  } else if (step === "result" && ok) {
    body = (
      <>
        <div style={{ textAlign: "center", padding: "10px 6px 4px" }}>
          <span style={{ width: 48, height: 48, borderRadius: "50%", background: "#EAF7EF",
            display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
            <svg viewBox="0 0 20 20" style={{ width: 24, height: 24 }}>
              <path d="M5 10.5 8.5 14 15 6.5" fill="none" stroke="#1C9E5A" strokeWidth="2.2"
                strokeLinecap="round" strokeLinejoin="round" /></svg>
          </span>
          <div style={{ fontSize: 15, fontWeight: 650, marginTop: 12 }}>迁移完成 · 探针验收通过</div>
          <div style={{ fontSize: 12.5, color: "#6B7682", marginTop: 5 }}>
            {result.msg_count} 条消息已写入 {TOOL_NAME[target]},源会话保持不变。</div>
        </div>
        <div style={{ marginTop: 18 }}>
          <CmdRow cmd={result.resume} head={`在 ${TOOL_NAME[target]} 中接续`} />
        </div>
        <button className="fbtn" style={{ width: "100%", height: 34, marginTop: 10, fontSize: 12.5 }}
          onClick={() => openTerminal(result.resume)}>在终端打开</button>
      </>
    );
  } else if (fail) {
    body = (
      <>
        <div style={{ border: "1px solid #EBCBC7", background: "#FDF3F1", borderRadius: 11,
          padding: "16px 18px", display: "flex", gap: 13 }}>
          <span style={{ width: 38, height: 38, flex: "none", borderRadius: "50%", background: "#FBE4E1",
            display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
            <svg viewBox="0 0 16 16" style={{ width: 18, height: 18 }}>
              <line x1="4" y1="4" x2="12" y2="12" stroke="#C4564C" strokeWidth="1.8" strokeLinecap="round" />
              <line x1="12" y1="4" x2="4" y2="12" stroke="#C4564C" strokeWidth="1.8" strokeLinecap="round" /></svg>
          </span>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 14, fontWeight: 650, color: "#8A3E37" }}>迁移失败 · 探针未通过</div>
            <div style={{ fontSize: 12.5, color: "#96524B", marginTop: 5, lineHeight: 1.5 }}>
              已自动回滚,未在 {TOOL_NAME[target]} 保留任何产物。源会话完好,你可以改用上下文摘要继续。</div>
            {(error || result?.probe?.detail) && (
              <pre className="mono selectable fscroll" style={{ margin: "8px 0 0", fontSize: 11,
                color: "#96524B", whiteSpace: "pre-wrap", maxHeight: 120, overflow: "auto" }}>
                {error || result.probe.detail}</pre>
            )}
          </div>
        </div>
        <button className="fbtn" style={{ width: "100%", height: 36, marginTop: 14, fontSize: 13 }}
          onClick={doHandoff} disabled={handoffBusy}>
          {handoffBusy ? "正在生成上下文摘要…" : "使用上下文摘要继续"}</button>
        {handoff && (
          <div style={{ marginTop: 12, border: "1px solid #E4E9EE", borderRadius: 10,
            overflow: "hidden", animation: "ffade .2s ease" }}>
            <div style={{ padding: "9px 13px", background: "#F4F7F9", borderBottom: "1px solid #E8ECF0",
              fontSize: 11.5, color: "#8A939D", fontWeight: 600 }}>上下文摘要预览</div>
            <div className="fscroll selectable" style={{ padding: "12px 14px", fontSize: 12,
              color: "#40494F", lineHeight: 1.6, maxHeight: 180, overflowY: "auto",
              whiteSpace: "pre-wrap" }}>{handoff.preview}</div>
            <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "11px 14px",
              borderTop: "1px solid #F0F3F6", background: "#F8FAFB" }}>
              <code className="mono selectable" style={{ flex: 1, fontSize: 12.5, color: "#334155",
                whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{handoff.command}</code>
              <button className="fbtn" onClick={() => openTerminal(handoff.command)}>在终端打开</button>
            </div>
          </div>
        )}
      </>
    );
  }

  const canBack = step === "preview" || step === "confirm";
  const canNext = step === "target" ? !!target
    : step === "preview" ? !!d
    : step === "confirm";

  return (
    <Sheet width={720} maxHeight={800} onClose={step === "writing" ? undefined : onClose}>
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid #E8ECF0",
        display: "flex", alignItems: "center", gap: 12 }}>
        <div style={{ fontSize: 14.5, fontWeight: 650 }}>迁移会话</div>
        <StepsHeader step={step} />
        <div style={{ flex: 1 }} />
        {step !== "writing" &&
          <a onClick={onClose} style={{ color: "#9AA3AD", fontSize: 18, lineHeight: 1 }}>×</a>}
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: 20,
        animation: "fslide .16s ease" }} key={step}>
        {body}
      </div>
      {step !== "writing" && (
        <div style={{ flex: "none", padding: "13px 20px", borderTop: "1px solid #E8ECF0",
          display: "flex", alignItems: "center", gap: 10 }}>
          {canBack && <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={back}>上一步</button>}
          <div style={{ flex: 1 }} />
          {step !== "result" && (
            <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onClose}>取消</button>)}
          {step !== "result" ? (
            <button className="fbtn-primary" style={{ height: 34, padding: "0 18px", fontSize: 13 }}
              disabled={!canNext} onClick={next}>
              {step === "confirm" ? "开始迁移" : "下一步"}</button>
          ) : (
            <button className="fbtn-primary" style={{ height: 34, padding: "0 18px", fontSize: 13 }}
              onClick={onClose}>完成</button>
          )}
        </div>
      )}
    </Sheet>
  );
}
