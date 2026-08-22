// 迁移向导:目标 → 损耗影响 → 目标会话预览 → 确认 → 写入 → 结果
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { engine, openTerminal } from "../../platform/desktop/client.js";
import { operations } from "../operations/public.js";
import {
  TOOL_NAME,
  agentsWithCapability,
  supportsAgentCapability,
} from "../../shared/contracts/tools.js";
import { ACCENT } from "../../shared/ui/toolDisplay.js";
import { sessionRef } from "../browser/public.js";
import { useFeature } from "../../shared/capabilities/features.jsx";
import { CheckBadge, Spinner, ToolIcon } from "../../shared/ui/icons.jsx";
import { CheckSquare, CmdRow, LossCols, Sheet } from "../../shared/ui/primitives.jsx";
import { probeFailed, probeText } from "../../shared/contracts/events.js";
import MigrationSessionPreview from "./MigrationSessionPreview.jsx";
import StepsHeader from "./SheetSteps.jsx";
import ProbeModelPicker from "./ProbeModelPicker.jsx";
import TransferTargetStep from "./TransferTargetStep.jsx";
import { canFallBackToResume } from "./resumeElsewhere.js";
import {
  matchingMigrationPlan,
  migrationPlanInput,
  migrationPlanKey,
} from "./migrationOperation.js";

const ORDER = ["target", "impact", "preview", "confirm", "result"];

export default function MigrateSheet({
  meta, scope, env, defaultProbe, terminalApp,
  onClose, onDone, onResumeElsewhere,
}) {
  const { t } = useTranslation();
  // 「续聊到」还在特性开关后面:关着时迁移失败只有重试,没有第二条出路。
  const resumeEnabled = useFeature("handoff");
  const targets = agentsWithCapability("migration-target")
    .filter(tool => tool !== meta.tool);
  const [step, setStep] = useState("target");
  const [target, setTarget] = useState(targets[0] || "");
  const [probeOn, setProbeOn] = useState(
    !!defaultProbe && supportsAgentCapability(targets[0], "probe"),
  );
  const [planned, setPlanned] = useState(null);
  const [dryErr, setDryErr] = useState(null);
  const [dryBusy, setDryBusy] = useState(false);
  const [result, setResult] = useState(null);
  const [error, setError] = useState(null);
  const [wroteFirst, setWroteFirst] = useState(false);
  const [modelCatalog, setModelCatalog] = useState({}); // { [tool]: catalog }
  const [modelLoad, setModelLoad] = useState({});
  const [modelErr, setModelErr] = useState({});
  const [probeModel, setProbeModel] = useState({});     // { [tool]: id }
  const [probeCustom, setProbeCustom] = useState({});   // { [tool]: free text }
  const canProbeTarget = supportsAgentCapability(target, "probe");
  const doneRef = useRef(false);
  const plannedRef = useRef(null);
  const planRequest = useRef(0);
  const ref = sessionRef(meta);

  const scopeLabel = scope ? t("migration:target.scopeToTurn", { n: scope }) : t("migration:target.scopeFull");
  const resolvedProbeModel = (probeCustom[target] || "").trim()
    || (probeModel[target] || "").trim()
    || undefined;
  const inputFor = (tgt = target) => migrationPlanInput({
    sourceTool: meta.tool,
    ref,
    targetTool: tgt,
    maxTurn: scope || undefined,
    probe: probeOn,
    probeModel: tgt === target ? resolvedProbeModel : undefined,
  });
  const currentInput = inputFor();
  const currentPlan = matchingMigrationPlan(planned, currentInput);
  const previewPlan = currentPlan || (
    planned?.input?.source_tool === currentInput.source_tool
    && planned.input.ref === currentInput.ref
    && planned.input.target_tool === currentInput.target_tool
    && planned.input.max_turn === currentInput.max_turn
      ? planned.plan : null
  );
  const d = previewPlan?.preview;
  const errorMessage = error => String(error?.message || error || t("errors:fallback"));
  // 引擎失败时会同时给出 error_message(人话原因)与 error_type(异常类名);
  // 只有拿不到人话原因时才退回类名,否则用户只能看到 SessionStoreUnavailableError。
  const failureText = error => {
    const errorType = error?.params?.error_type;
    if (errorType === "ConcurrentModificationError")
      return t("migration:result.failConcurrent", { tool: TOOL_NAME[meta.tool] });
    const detail = error?.params?.error_message;
    if (detail) return detail;
    return errorType ? `${errorMessage(error)} · ${errorType}` : errorMessage(error);
  };

  const rememberPlan = (key, input, plan) => {
    const value = { key, input, plan };
    plannedRef.current = value;
    setPlanned(value);
    return plan;
  };

  const ensurePlan = async input => {
    const cached = matchingMigrationPlan(plannedRef.current, input);
    if (cached) return cached;
    const plan = await operations.plan(input);
    return rememberPlan(migrationPlanKey(input), input, plan);
  };

  const loadDry = async tgt => {
    const request = ++planRequest.current;
    const input = inputFor(tgt);
    setDryErr(null);
    setDryBusy(true);
    try {
      const cached = matchingMigrationPlan(plannedRef.current, input);
      if (!cached) {
        const plan = await operations.plan(input);
        if (request === planRequest.current)
          rememberPlan(migrationPlanKey(input), input, plan);
      }
    } catch (e) {
      // 存错误对象而不是文案:「改用交接」要按 code 判断该不该出现。
      if (request === planRequest.current) setDryErr(e);
    } finally {
      if (request === planRequest.current) setDryBusy(false);
    }
  };

  const loadModels = async tgt => {
    if (!supportsAgentCapability(tgt, "models")) return;
    if (modelCatalog[tgt] || modelLoad[tgt]) return;
    setModelLoad(prev => ({ ...prev, [tgt]: true }));
    setModelErr(prev => ({ ...prev, [tgt]: null }));
    try {
      const r = await engine("models", { tool: tgt });
      setModelCatalog(prev => ({ ...prev, [tgt]: r }));
    } catch (e) {
      setModelErr(prev => ({ ...prev, [tgt]: errorMessage(e) }));
    }
    setModelLoad(prev => ({ ...prev, [tgt]: false }));
  };

  useEffect(() => {
    if (step === "confirm") loadModels(target);
  }, [step, target]);

  const next = () => {
    if (step === "target") { if (!currentPlan) loadDry(target); setStep("impact"); }
    else if (step === "impact") setStep("preview");
    else if (step === "preview") { loadModels(target); setStep("confirm"); }
    else if (step === "confirm") execute();
  };
  const back = () => {
    if (step === "impact") setStep("target");
    else if (step === "preview") setStep("impact");
    else if (step === "confirm") setStep("preview");
  };

  const execute = async () => {
    setStep("writing");
    setWroteFirst(false);
    setTimeout(() => setWroteFirst(true), 1500);
    try {
      const plan = await ensurePlan(currentInput);
      const applied = await operations.apply(plan);
      setResult(applied.result);
    } catch (e) { setError(failureText(e)); }
    setStep("result");
    if (!doneRef.current) { doneRef.current = true; onDone?.(); }
  };

  const ok = result && !probeFailed(result.probe) && result.session_id;
  const fail = step === "result" && !ok;
  // 「已自动回滚」只对真的回滚过的结果成立:写入前被门禁/并发检查拦下时
  // 目标端根本没被碰过,再说回滚会误导用户去目标端找残留。
  const rolledBack = !!result
    && (result.rolled_back === true || result.validation?.structure?.ok === false);
  const failTitleKey = rolledBack ? "failTitle"
    : result ? "failProbeTitle" : "failAbortedTitle";
  const failDescKey = rolledBack ? "failDesc"
    : result ? "failProbeDesc" : "failAbortedDesc";
  const installed = t => env?.[t]?.installed;

  let body = null;
  if (step === "target") {
    body = (
      <TransferTargetStep
        meta={meta}
        scopeLabel={scopeLabel}
        targets={targets}
        target={target}
        onTarget={tool => {
          setTarget(tool);
          if (!supportsAgentCapability(tool, "probe")) setProbeOn(false);
        }}
        env={env}
      />
    );
  } else if (step === "impact") {
    body = !d ? (
      <div style={{ padding: "60px 0", display: "flex", alignItems: "center", justifyContent: "center",
        gap: 10, color: "var(--tx4)", fontSize: 13 }}>
        {dryErr ? (<>
          <span style={{ color: "var(--err-deep)" }}>
            {t("migration:preview.failed", { error: errorMessage(dryErr) })}</span>
          <button className="fbtn" onClick={() => loadDry(target)}>{t("migration:preview.retry")}</button>
          {/* 迁移的前置条件没满足时,「续聊」是那条零写入、永远可行的退路:
              只复制一条指令,不切换面板步骤,用户拿着去任一装了 skill 的 agent 里粘贴即可 */}
          {resumeEnabled && canFallBackToResume(dryErr) && (
            <button className="fbtn-primary" onClick={() => onResumeElsewhere?.()}>
              {t("migration:resume.fallback")}
            </button>
          )}
        </>) : <><Spinner size={16} /> {t("migration:preview.loading")}</>}
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
          {t("migration:preview.impactTitle", { scope: scopeLabel })}</div>
        <div style={{ marginBottom: 16 }}><LossCols loss={d.loss} /></div>
        <div className="fcard" style={{ padding: "11px 14px",
          display: "flex", justifyContent: "space-between", alignItems: "center", fontSize: 12 }}>
          <span style={{ color: "var(--tx2)", fontWeight: 600 }}>{t("migration:preview.scaleLabel")}</span>
          <span className="mono" style={{ color: "var(--tx2)" }}>
            {t("migration:preview.scaleMeta", { msg: d.msg_count, tree: d.tree_count })}</span>
        </div>
      </>
    );
  } else if (step === "preview") {
    body = <MigrationSessionPreview preview={d?.preview} />;
  } else if (step === "confirm") {
    body = (
      <>
        <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "16px 18px" }}>
          <div style={{ fontSize: 13, fontWeight: 600, marginBottom: 12 }}>{t("migration:confirm.title")}</div>
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
        {canProbeTarget && <div style={{ border: "1px solid var(--line3)", borderRadius: 10, padding: "13px 15px",
          marginTop: 12, display: "flex", alignItems: "flex-start", gap: 11 }}>
          <label onClick={() => setProbeOn(v => !v)}
            style={{ display: "flex", alignItems: "center", gap: 8, cursor: "default", flex: "none",
              marginTop: 1 }}>
            <CheckSquare on={probeOn} accent={ACCENT} fg="#fff" />
            <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2)" }}>{t("migration:confirm.probeTitle")}</span>
          </label>
          <div style={{ fontSize: 11, color: "var(--tx3b)", lineHeight: 1.5 }}>
            {t("migration:confirm.probeDesc")}</div>
        </div>}
        {canProbeTarget && probeOn && (
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
          <div style={{ fontSize: 15, fontWeight: 600, marginTop: 12 }}>
            {result.validation?.runtime?.status === "passed"
              ? t("migration:result.doneBoth") : t("migration:result.doneStructure")}</div>
          <div style={{ fontSize: 12, color: "var(--tx3b)", marginTop: 5 }}>
            {t("migration:result.doneDesc", { n: result.msg_count, tool: TOOL_NAME[target] })}</div>
        </div>
        {supportsAgentCapability(target, "resume") && result.resume && (<>
          <div style={{ marginTop: 18 }}>
            <CmdRow cmd={result.resume} head={t("migration:result.continueIn", { tool: TOOL_NAME[target] })} />
          </div>
          <button className="fbtn" style={{ width: "100%", height: 34, marginTop: 10, fontSize: 12 }}
            onClick={() => openTerminal(result.resume, terminalApp)}>{t("migration:result.openTerminal")}</button>
        </>)}
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
            <div style={{ fontSize: 14, fontWeight: 600, color: "var(--err-text)" }}>{t(`migration:result.${failTitleKey}`)}</div>
            <div style={{ fontSize: 12, color: "var(--err-mut)", marginTop: 5, lineHeight: 1.5 }}>
              {t(`migration:result.${failDescKey}`, { tool: TOOL_NAME[target] })}
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
      </>
    );
  }

  const canBack = step === "impact" || step === "preview" || step === "confirm";
  const canNext = step === "target" ? !!target && installed(target)
    : step === "impact" || step === "preview" ? !!d
      : step === "confirm";

  return (
    <Sheet width={720} maxHeight={800} onClose={step === "writing" ? undefined : onClose}>
      <div style={{ flex: "none", padding: "15px 20px", borderBottom: "1px solid var(--line5)",
        display: "flex", alignItems: "center", gap: 12 }}>
        <div style={{ fontSize: 14, fontWeight: 600 }}>{t("migration:sheet.title")}</div>
        <StepsHeader step={step} order={ORDER} t={t} />
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
