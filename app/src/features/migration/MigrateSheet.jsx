// 迁移向导:目标 → 预演(dry_run) → 确认 → 写入 → 结果(成功/失败+摘要兜底)
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { openTerminal, rpc } from "../../api/transport/rpc.js";
import { TOOL_NAME, TOOLS } from "../../api/contract/tools.js";
import { ACCENT } from "../../domain/tools/toolDisplay.js";
import { sessionRef } from "../../domain/sessions/sessionModel.js";
import { CheckBadge, Spinner, ToolIcon } from "../../components/ui/icons.jsx";
import { CheckSquare, CmdRow, LossCols, Sheet } from "../../components/ui/primitives.jsx";
import { probeFailed, probeText } from "../../api/contract/events.js";

const ORDER = ["target", "preview", "confirm", "result"];

function StepsHeader({ step, t }) {
  const labels = {
    target: t("migration:steps.target"),
    preview: t("migration:steps.preview"),
    confirm: t("migration:steps.confirm"),
    result: step === "writing" ? t("migration:steps.writing") : t("migration:steps.result"),
  };
  const cur = ORDER.indexOf(step === "writing" ? "result" : step);
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 7, marginLeft: 6 }}>
      {ORDER.map((s, i) => (
        <span key={s} style={{ display: "inline-flex", alignItems: "center", gap: 7 }}>
          <span style={{ fontSize: 11, fontWeight: 600,
            color: i === cur ? ACCENT : i < cur ? "var(--tx3)" : "var(--line-strong)" }}>{labels[s]}</span>
          {i < ORDER.length - 1 && <span style={{ color: "var(--line-strong)", fontSize: 11 }}>›</span>}
        </span>
      ))}
    </div>
  );
}

function ProbeModelPicker({ catalog, loading, err, selected, custom, onSelect, onCustom, t }) {
  const models = catalog?.models || [];
  const filterable = models.length > 12;
  const [q, setQ] = useState("");
  const qn = q.trim().toLowerCase();
  const shown = !filterable || !qn ? models
    : models.filter(m => (m.id + " " + (m.label || "")).toLowerCase().includes(qn));
  const srcHint = {
    cli: t("migration:probeModel.sourceCli"),
    alias: t("migration:probeModel.sourceAlias"),
    fallback: t("migration:probeModel.sourceFallback"),
    cache: t("migration:probeModel.sourceCache"),
    user: t("migration:probeModel.sourceUser"),
  }[catalog?.source] || "";

  return (
    <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "14px 16px", marginTop: 12 }}>
      <div style={{ display: "flex", justifyContent: "space-between", gap: 12, marginBottom: 8 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2)" }}>{t("migration:probeModel.title")}</div>
        <div style={{ fontSize: 11, color: "var(--tx4)" }}>
          {loading ? t("migration:probeModel.loading") : srcHint}
        </div>
      </div>
      <div style={{ fontSize: 11, color: "var(--tx3b)", marginBottom: 10, lineHeight: 1.45 }}>
        {t("migration:probeModel.hint")}
      </div>
      {err && <div style={{ fontSize: 11, color: "var(--err-deep)", marginBottom: 8 }}>{t("migration:probeModel.loadFailed", { error: err })}</div>}
      {catalog?.error && !err && (
        <div style={{ fontSize: 11, color: "var(--err-mut)", marginBottom: 8 }}>
          {t("migration:probeModel.discoverWarn", { error: catalog.error })}
        </div>
      )}
      {filterable && (
        <input value={q} onChange={e => setQ(e.target.value)} placeholder={t("migration:probeModel.filterPlaceholder")}
          style={{ width: "100%", height: 32, border: "1px solid var(--line)", borderRadius: 8,
            padding: "0 10px", fontSize: 12, marginBottom: 8 }} />
      )}
      <select value={selected} onChange={e => onSelect(e.target.value)}
        disabled={loading}
        style={{ width: "100%", height: 34, border: "1px solid var(--line)", borderRadius: 8,
          padding: "0 10px", fontSize: 12, background: "var(--surface)", color: "var(--tx2)" }}>
        <option value="">{t("migration:probeModel.toolDefault", { suffix: catalog?.default ? ` (${catalog.default})` : "" })}</option>
        {shown.map(m => (
          <option key={m.id} value={m.id}>{m.label || m.id}</option>
        ))}
      </select>
      {catalog?.allow_custom !== false && (
        <input value={custom} onChange={e => onCustom(e.target.value)}
          placeholder={t("migration:probeModel.customPlaceholder")}
          style={{ width: "100%", height: 32, border: "1px solid var(--line)", borderRadius: 8,
            padding: "0 10px", fontSize: 12, marginTop: 8 }} />
      )}
    </div>
  );
}

export default function MigrateSheet({ meta, scope, env, defaultProbe, onClose, onDone }) {
  const { t } = useTranslation();
  const targets = TOOLS.filter(t2 => t2 !== meta.tool);
  const [step, setStep] = useState("target");
  const [target, setTarget] = useState(targets[0]);
  const [probeOn, setProbeOn] = useState(!!defaultProbe);
  const [dry, setDry] = useState(null);        // { [target]: result }
  const [dryErr, setDryErr] = useState(null);
  const [result, setResult] = useState(null);
  const [error, setError] = useState(null);
  const [wroteFirst, setWroteFirst] = useState(false);
  const [handoff, setHandoff] = useState(null);
  const [handoffBusy, setHandoffBusy] = useState(false);
  const [modelCatalog, setModelCatalog] = useState({}); // { [tool]: catalog }
  const [modelLoad, setModelLoad] = useState({});
  const [modelErr, setModelErr] = useState({});
  const [probeModel, setProbeModel] = useState({});     // { [tool]: id }
  const [probeCustom, setProbeCustom] = useState({});   // { [tool]: free text }
  const doneRef = useRef(false);
  const ref = sessionRef(meta);

  const d = dry?.[target];
  const scopeLabel = scope ? t("migration:target.scopeToTurn", { n: scope }) : t("migration:target.scopeFull");
  const resolvedProbeModel = (probeCustom[target] || "").trim()
    || (probeModel[target] || "").trim()
    || undefined;

  const loadDry = async tgt => {
    setDryErr(null);
    try {
      const r = await rpc("migrate", { src: meta.tool, dst: tgt, ref,
        dry_run: true, max_turn: scope || undefined });
      setDry(prev => ({ ...prev, [tgt]: r }));
    } catch (e) { setDryErr(e.message); }
  };

  const loadModels = async tgt => {
    if (modelCatalog[tgt] || modelLoad[tgt]) return;
    setModelLoad(prev => ({ ...prev, [tgt]: true }));
    setModelErr(prev => ({ ...prev, [tgt]: null }));
    try {
      const r = await rpc("models", { tool: tgt });
      setModelCatalog(prev => ({ ...prev, [tgt]: r }));
    } catch (e) {
      setModelErr(prev => ({ ...prev, [tgt]: e.message }));
    }
    setModelLoad(prev => ({ ...prev, [tgt]: false }));
  };

  useEffect(() => {
    if (step === "confirm" || step === "preview") loadModels(target);
  }, [step, target]);

  const next = () => {
    if (step === "target") { if (!dry?.[target]) loadDry(target); setStep("preview"); }
    else if (step === "preview") { loadModels(target); setStep("confirm"); }
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
        max_turn: scope || undefined,
        probe: probeOn,
        probe_model: probeOn ? resolvedProbeModel : undefined });
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

  const ok = result && !probeFailed(result.probe) && result.session_id;
  const fail = step === "result" && !ok;
  const installed = t => env?.[t]?.installed;

  let body = null;
  if (step === "target") {
    body = (
      <>
        <div style={{ fontSize: 13, color: "var(--tx3b)", marginBottom: 6 }}>
          {t("migration:target.sourceSession")} <b style={{ color: "var(--tx2)" }}>{meta.title || meta.id}</b> · {scopeLabel}</div>
        <div style={{ fontSize: 12, color: "var(--tx4)", marginBottom: 14 }}>
          {t("migration:target.chooseHint")}</div>
        {targets.map(t2 => {
          const on = target === t2;
          const inst = installed(t2);
          return (
            <div key={t2} onClick={() => inst && setTarget(t2)}
              style={{ display: "flex", alignItems: "center", gap: 12, padding: "13px 14px",
                border: `1.5px solid ${on ? ACCENT : "var(--line3)"}`, background: on ? "var(--acc-soft4)" : "var(--surface)",
                borderRadius: 10, marginBottom: 9, cursor: inst ? "pointer" : "not-allowed",
                opacity: inst ? 1 : 0.55 }}>
              <ToolIcon tool={t2} size={32} />
              <div style={{ flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx2)" }}>{TOOL_NAME[t2]}</div>
                <div style={{ fontSize: 11, color: "var(--tx4)" }}>
                  {inst ? t("migration:target.installedMeta", { version: env[t2].version || "?", tool: t2 })
                    : t("migration:target.notInstalled")}
                </div>
              </div>
              <span style={{ width: 18, height: 18, borderRadius: "50%",
                border: `2px solid ${on ? ACCENT : "var(--line-strong)"}`, display: "inline-flex",
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
        gap: 10, color: "var(--tx4)", fontSize: 13 }}>
        {dryErr ? <span style={{ color: "var(--err-deep)" }}>{t("migration:preview.failed", { error: dryErr })}</span>
          : <><Spinner size={16} /> {t("migration:preview.loading")}</>}
      </div>
    ) : (
      <>
        <div style={{ display: "flex", alignItems: "center", gap: 9, marginBottom: 14 }}>
          <ToolIcon tool={meta.tool} size={24} />
          <span style={{ color: "var(--tx4)", fontSize: 12 }}>{TOOL_NAME[meta.tool]}</span>
          <span style={{ color: "var(--line-strong)" }}>→</span>
          <ToolIcon tool={target} size={26} />
          <span style={{ fontSize: 13, fontWeight: 600, color: "var(--tx2)" }}>{TOOL_NAME[target]}</span>
        </div>
        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--tx3b)", marginBottom: 8 }}>
          {t("migration:preview.lossTitle", { scope: scopeLabel })}</div>
        <div style={{ marginBottom: 16 }}><LossCols loss={d.loss} /></div>
        <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "13px 15px",
          display: "flex", justifyContent: "space-between", alignItems: "center", fontSize: 12 }}>
          <span style={{ color: "var(--tx2)", fontWeight: 600 }}>{t("migration:preview.scaleLabel")}</span>
          <span className="mono" style={{ color: "var(--tx2)" }}>
            {t("migration:preview.scaleMeta", { msg: d.msg_count, tree: d.tree_count })}</span>
        </div>
      </>
    );
  } else if (step === "confirm") {
    body = (
      <>
        <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "16px 18px" }}>
          <div style={{ fontSize: 13, fontWeight: 650, marginBottom: 12 }}>{t("migration:confirm.title")}</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 9, fontSize: 12 }}>
            {[["target", TOOL_NAME[target], true],
              ["scope", d ? t("migration:confirm.scopeWithCount", { scope: scopeLabel, n: d.msg_count }) : scopeLabel],
              ["structureCheck", t("migration:confirm.structureAlways")],
              ["runtimeProbe", probeOn
                ? t("migration:confirm.probeOn", { model: resolvedProbeModel || t("migration:confirm.probeOff") })
                : t("migration:confirm.probeOff")],
            ].map(([k, v, bold], i) => (
              <div key={i} style={{ display: "flex", justifyContent: "space-between", gap: 20 }}>
                <span style={{ color: "var(--tx4)", flex: "none" }}>{t(`migration:confirm.${k}`)}</span>
                <span style={{ color: "var(--tx2)", fontWeight: bold ? 600 : 400, textAlign: "right" }}>{v}</span>
              </div>
            ))}
          </div>
        </div>
        <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "13px 15px",
          marginTop: 12, display: "flex", alignItems: "flex-start", gap: 11 }}>
          <label onClick={() => setProbeOn(v => !v)}
            style={{ display: "flex", alignItems: "center", gap: 8, cursor: "default", flex: "none",
              marginTop: 1 }}>
            <CheckSquare on={probeOn} accent={ACCENT} fg="#fff" />
            <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2)" }}>{t("migration:confirm.probeTitle")}</span>
          </label>
          <div style={{ fontSize: 11, color: "var(--tx3b)", lineHeight: 1.5 }}>
            {t("migration:confirm.probeDesc")}</div>
        </div>
        {probeOn && (
          <ProbeModelPicker
            catalog={modelCatalog[target]}
            loading={!!modelLoad[target]}
            err={modelErr[target]}
            selected={probeModel[target] || ""}
            custom={probeCustom[target] || ""}
            onSelect={v => setProbeModel(prev => ({ ...prev, [target]: v }))}
            onCustom={v => setProbeCustom(prev => ({ ...prev, [target]: v }))}
            t={t}
          />
        )}
        <div style={{ fontSize: 12, color: "var(--tx3b)", margin: "14px 0 0", lineHeight: 1.55 }}>
          {t("migration:confirm.epilogue", { probe: probeOn ? t("migration:confirm.probeEpilogue") : "" })}</div>
      </>
    );
  } else if (step === "writing") {
    const items = [
      { label: t("migration:writing.writeTarget", { tool: TOOL_NAME[target] }), state: wroteFirst ? "done" : "spin" },
      { label: probeOn ? t("migration:writing.structureProbe") : t("migration:writing.structureOnly"),
        state: wroteFirst ? "spin" : "wait" },
    ];
    body = (
      <div style={{ padding: "24px 6px" }}>
        {items.map((p, i) => (
          <div key={i} style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 0" }}>
            {p.state === "spin" && <Spinner size={18} track="var(--line)" />}
            {p.state === "done" && <CheckBadge />}
            {p.state === "wait" && <span style={{ width: 18, height: 18, borderRadius: "50%",
              border: "2px solid var(--line)", flex: "none" }} />}
            <span style={{ fontSize: 13, color: p.state === "spin" ? ACCENT : "var(--tx2)",
              fontWeight: p.state === "spin" ? 600 : 500 }}>{p.label}</span>
          </div>
        ))}
      </div>
    );
  } else if (step === "result" && ok) {
    body = (
      <>
        <div style={{ textAlign: "center", padding: "10px 6px 4px" }}>
          <span style={{ width: 48, height: 48, borderRadius: "50%", background: "var(--ok-bg)",
            display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
            <svg viewBox="0 0 20 20" style={{ width: 24, height: 24 }}>
              <path d="M5 10.5 8.5 14 15 6.5" fill="none" stroke="var(--ok)" strokeWidth="2.2"
                strokeLinecap="round" strokeLinejoin="round" /></svg>
          </span>
          <div style={{ fontSize: 15, fontWeight: 650, marginTop: 12 }}>
            {result.validation?.runtime?.status === "passed"
              ? t("migration:result.doneBoth") : t("migration:result.doneStructure")}</div>
          <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 5 }}>
            {t("migration:result.doneDesc", { n: result.msg_count, tool: TOOL_NAME[target] })}</div>
        </div>
        <div style={{ marginTop: 18 }}>
          <CmdRow cmd={result.resume} head={t("migration:result.handoffIn", { tool: TOOL_NAME[target] })} />
        </div>
        <button className="fbtn" style={{ width: "100%", height: 34, marginTop: 10, fontSize: 12 }}
          onClick={() => openTerminal(result.resume)}>{t("migration:result.openTerminal")}</button>
      </>
    );
  } else if (fail) {
    body = (
      <>
        <div style={{ border: "1px solid var(--err-line)", background: "var(--err-bg)", borderRadius: 10,
          padding: "16px 18px", display: "flex", gap: 13 }}>
          <span style={{ width: 38, height: 38, flex: "none", borderRadius: "50%", background: "var(--err-bg3)",
            display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
            <svg viewBox="0 0 16 16" style={{ width: 18, height: 18 }}>
              <line x1="4" y1="4" x2="12" y2="12" stroke="var(--err2)" strokeWidth="1.8" strokeLinecap="round" />
              <line x1="12" y1="4" x2="4" y2="12" stroke="var(--err2)" strokeWidth="1.8" strokeLinecap="round" /></svg>
          </span>
          <div style={{ minWidth: 0 }}>
            <div style={{ fontSize: 14, fontWeight: 650, color: "var(--err-text)" }}>{t("migration:result.failTitle")}</div>
            <div style={{ fontSize: 12, color: "var(--err-mut)", marginTop: 5, lineHeight: 1.5 }}>
              {t("migration:result.failDesc", { tool: TOOL_NAME[target] })}
              {(result?.probe?.model || result?.probe_model) && (
                <>{t("migration:result.failDescProbe", { model: result.probe?.model || result.probe_model })}</>
              )}
            </div>
            {(error || probeText(result?.probe)) && (
              <pre className="mono selectable fscroll" style={{ margin: "10px 0 0", fontSize: 11,
                color: "var(--err-pre)", whiteSpace: "pre-wrap", maxHeight: 280, overflow: "auto",
                background: "var(--err-bg4)", border: "1px solid var(--err-line)", borderRadius: 8, padding: "10px 12px",
                lineHeight: 1.5 }}>
                {error || probeText(result.probe)}</pre>
            )}
          </div>
        </div>
        <button className="fbtn" style={{ width: "100%", height: 36, marginTop: 14, fontSize: 13 }}
          onClick={doHandoff} disabled={handoffBusy}>
          {handoffBusy ? t("migration:result.handoffBusy") : t("migration:result.useHandoff")}</button>
        {handoff && (
          <div style={{ marginTop: 12, border: "1px solid var(--line3)", borderRadius: 10,
            overflow: "hidden" }}>
            <div style={{ padding: "9px 13px", background: "var(--fill2)", borderBottom: "1px solid var(--line5)",
              fontSize: 11, color: "var(--tx4)", fontWeight: 600 }}>{t("migration:result.handoffPreview")}</div>
            <div className="fscroll selectable" style={{ padding: "12px 14px", fontSize: 12,
              color: "var(--tx2b)", lineHeight: 1.6, maxHeight: 180, overflowY: "auto",
              whiteSpace: "pre-wrap" }}>{handoff.preview}</div>
            <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "11px 14px",
              borderTop: "1px solid var(--line6)", background: "var(--fill)" }}>
              <code className="mono selectable" style={{ flex: 1, fontSize: 12, color: "var(--tx2)",
                whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{handoff.command}</code>
              <button className="fbtn" onClick={() => openTerminal(handoff.command)}>{t("migration:result.openTerminal")}</button>
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
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid var(--line5)",
        display: "flex", alignItems: "center", gap: 12 }}>
        <div style={{ fontSize: 14, fontWeight: 650 }}>{t("migration:sheet.title")}</div>
        <StepsHeader step={step} t={t} />
        <div style={{ flex: 1 }} />
        {step !== "writing" &&
          <a onClick={onClose} style={{ color: "var(--tx5)", fontSize: 18, lineHeight: 1 }}>×</a>}
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: 20,
         }} key={step}>
        {body}
      </div>
      {step !== "writing" && (
        <div style={{ flex: "none", padding: "13px 20px", borderTop: "1px solid var(--line5)",
          display: "flex", alignItems: "center", gap: 10 }}>
          {canBack && <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={back}>{t("migration:sheet.back")}</button>}
          <div style={{ flex: 1 }} />
          {step !== "result" && (
            <button className="fbtn" style={{ height: 34, fontSize: 13 }} onClick={onClose}>{t("migration:sheet.cancel")}</button>)}
          {step !== "result" ? (
            <button className="fbtn-primary" style={{ height: 34, padding: "0 18px", fontSize: 13 }}
              disabled={!canNext} onClick={next}>
              {step === "confirm" ? t("migration:sheet.start") : t("migration:sheet.next")}</button>
          ) : (
            <button className="fbtn-primary" style={{ height: 34, padding: "0 18px", fontSize: 13 }}
              onClick={onClose}>{t("migration:sheet.done")}</button>
          )}
        </div>
      )}
    </Sheet>
  );
}
