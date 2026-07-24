import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  runtime,
  trustedEngine,
} from "../../platform/desktop/client.js";
import { sessionRef } from "../browser/sessionModel.js";

const wait = milliseconds => new Promise(resolve =>
  globalThis.setTimeout(resolve, milliseconds));

async function generateOrganizationProposal(sessions, locale, onStarted) {
  let state = await runtime("organization.start", {
    locale,
    sessions: sessions.slice(0, 50).map(session => ({
      tool: session.tool,
      id: session.id,
      ref: sessionRef(session),
      title: session.title,
      project: session.project,
      ...(session.updated_at ? { updated_at: String(session.updated_at) } : {}),
    })),
  });
  if (typeof state?.job_id !== "string") {
    throw new Error("organization job id missing");
  }
  onStarted(state.job_id);
  while (state.status === "running") {
    await wait(200);
    state = await runtime("organization.status", {
      job_id: state.job_id,
    });
  }
  if (state.status === "completed") return state.result;
  throw new Error(
    state.error?.message || `organization job ${state.status}`,
  );
}

function editableTargets(proposal) {
  return Object.fromEntries((proposal?.targets || []).map(target => [
    `${target.tool}\0${target.id}`,
    {
      ...target.suggested,
      tagsText: (target.suggested.tags || []).join(", "),
    },
  ]));
}

export default function OrganizationPanel({ sessions, onClose, onApplied }) {
  const { t, i18n } = useTranslation();
  const [proposal, setProposal] = useState(null);
  const [drafts, setDrafts] = useState({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const activeJob = useRef(null);

  const targets = useMemo(() => sessions.slice(0, 50), [sessions]);
  const scopeKey = JSON.stringify(
    targets.map(target => [target.tool, target.id]),
  );
  const adopt = value => {
    setProposal(value);
    setDrafts(editableTargets(value));
  };

  useEffect(() => {
    const allowed = new Set(targets.map(target => `${target.tool}\0${target.id}`));
    trustedEngine("organization_proposals_list", { status: "pending" })
      .then(list => {
        const match = list?.find(item => item.targets?.every(target =>
          allowed.has(`${target.tool}\0${target.id}`)));
        if (match) adopt(match);
      })
      .catch(() => {});
  }, [scopeKey]);

  useEffect(() => () => {
    if (activeJob.current) {
      void runtime("organization.cancel", {
        job_id: activeJob.current,
      }).catch(() => {});
    }
  }, []);

  const closePanel = () => {
    const jobId = activeJob.current;
    activeJob.current = null;
    if (jobId) {
      void runtime("organization.cancel", { job_id: jobId })
        .catch(() => {});
    }
    onClose();
  };

  const generate = async () => {
    setBusy(true); setError("");
    try {
      adopt(await generateOrganizationProposal(
        targets,
        i18n.language,
        jobId => {
          activeJob.current = jobId;
        },
      ));
    } catch (error2) {
      setError(error2.message || String(error2));
    } finally {
      activeJob.current = null;
      setBusy(false);
    }
  };

  const saveEdits = async () => {
    setBusy(true); setError("");
    try {
      const changes = proposal.targets.map(target => {
        const draft = drafts[`${target.tool}\0${target.id}`];
        const { tagsText: _tagsText, ...suggested } = draft;
        return {
          tool: target.tool, id: target.id,
          suggested: {
            ...suggested,
            tags: draft.tagsText.split(/[,，]/).map(value => value.trim()).filter(Boolean),
          },
        };
      });
      adopt(await trustedEngine("organization_proposal_modify", {
        proposal_id: proposal.proposal_id, changes,
      }));
    } catch (error2) {
      setError(error2.message || String(error2));
    } finally {
      setBusy(false);
    }
  };
  const decide = async decision => {
    setBusy(true); setError("");
    try {
      const value = await trustedEngine("organization_proposal_decide", {
        proposal_id: proposal.proposal_id, decision,
      });
      adopt(value);
      if (decision === "approve") onApplied?.(value);
    } catch (error2) {
      setError(error2.message || String(error2));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div onMouseDown={event => {
      if (event.target === event.currentTarget) closePanel();
    }}
      style={{ position: "absolute", inset: 0, zIndex: 58, background: "var(--scrim)",
        display: "flex", alignItems: "center", justifyContent: "center" }}>
      <div style={{ width: "min(760px, calc(100vw - 48px))",
        height: "min(650px, calc(100vh - 56px))", background: "var(--surface)",
        border: "1px solid var(--line)", borderRadius: 14, boxShadow: "var(--shadow-sheet)",
        display: "flex", flexDirection: "column", overflow: "hidden" }}>
        <div style={{ height: 52, display: "flex", alignItems: "center", gap: 10,
          padding: "0 18px", borderBottom: "1px solid var(--line4)" }}>
          <strong style={{ fontSize: 14 }}>{t("organizing:title")}</strong>
          <span style={{ color: "var(--tx5)", fontSize: 11 }}>
            {t("organizing:scope", {
              n: proposal?.targets?.length || targets.length,
            })}</span>
          <span style={{ flex: 1 }} />
          <button className="fbtn" onClick={closePanel}>
            {t("organizing:close")}
          </button>
        </div>
        <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: 18 }}>
          {!proposal && (
            <div style={{ maxWidth: 520, margin: "70px auto", textAlign: "center" }}>
              <div style={{ fontSize: 13, color: "var(--tx2)", lineHeight: 1.6 }}>
                {t("organizing:description")}</div>
              <button className="fbtn-primary" disabled={busy || !targets.length}
                onClick={generate} style={{ marginTop: 16 }}>
                {busy ? t("organizing:generating") : t("organizing:generate")}
              </button>
            </div>
          )}
          {proposal && (
            <div style={{ display: "grid", gap: 12 }}>
              <div style={{ fontSize: 11, color: "var(--tx5)" }}>
                {t(`organizing:status.${proposal.status}`)}
                {proposal.cache_hit ? ` · ${t("organizing:cacheHit")}` : ""}
              </div>
              {proposal.targets.map(target => {
                const key = `${target.tool}\0${target.id}`;
                const draft = drafts[key] || {};
                return (
                  <div key={key} className="fcard" style={{ padding: 14 }}>
                    <div style={{ fontSize: 11, color: "var(--tx5)" }}>
                      {target.tool} · {target.id}</div>
                    <div style={{ marginTop: 9, padding: "9px 10px",
                      border: "1px solid var(--line4)", borderRadius: 8,
                      background: "var(--bg2)" }}>
                      <div style={{ fontSize: 10.5, fontWeight: 650,
                        color: "var(--tx4)" }}>
                        {t("organizing:current")}
                      </div>
                      <div style={{ marginTop: 5, fontSize: 11.5, color: "var(--tx2)" }}>
                        {target.current?.name || t("organizing:empty")}
                      </div>
                      <div style={{ marginTop: 3, fontSize: 11, lineHeight: 1.5,
                        color: "var(--tx4)" }}>
                        {target.current?.summary || t("organizing:noSummary")}
                      </div>
                      <div style={{ marginTop: 4, fontSize: 10.5, color: "var(--tx5)" }}>
                        {(target.current?.tags || []).length
                          ? target.current.tags.join(" · ")
                          : t("organizing:noTags")}
                        {target.current?.cluster_name
                          ? ` · ${t("organizing:cluster")}: ${target.current.cluster_name}`
                          : ""}
                        {target.current?.dead_candidate
                          ? ` · ${t("organizing:dead")}` : ""}
                      </div>
                    </div>
                    <div style={{ marginTop: 10, fontSize: 10.5, fontWeight: 650,
                      color: "var(--tx4)" }}>
                      {t("organizing:suggested")}
                    </div>
                    <label style={{ display: "block", fontSize: 11, color: "var(--tx4)",
                      marginTop: 6 }}>{t("organizing:name")}
                      <input className="finput" value={draft.name || ""}
                        disabled={proposal.status !== "pending"}
                        onChange={event => setDrafts(value => ({
                          ...value, [key]: { ...draft, name: event.target.value },
                        }))}
                        style={{ display: "block", width: "100%", marginTop: 4 }} />
                    </label>
                    <label style={{ display: "block", fontSize: 11, color: "var(--tx4)",
                      marginTop: 8 }}>{t("organizing:summary")}
                      <textarea className="finput" rows={3} value={draft.summary || ""}
                        disabled={proposal.status !== "pending"}
                        onChange={event => setDrafts(value => ({
                          ...value, [key]: { ...draft, summary: event.target.value },
                        }))}
                        style={{ display: "block", width: "100%", resize: "vertical",
                          marginTop: 4 }} />
                    </label>
                    <label style={{ display: "block", fontSize: 11, color: "var(--tx4)",
                      marginTop: 8 }}>{t("organizing:tags")}
                      <input className="finput" value={draft.tagsText || ""}
                        disabled={proposal.status !== "pending"}
                        onChange={event => setDrafts(value => ({
                          ...value, [key]: { ...draft, tagsText: event.target.value },
                        }))}
                        style={{ display: "block", width: "100%", marginTop: 4 }} />
                    </label>
                    <div style={{ marginTop: 8, fontSize: 11, color: "var(--tx4)" }}>
                      {draft.cluster_name && `${t("organizing:cluster")}: ${draft.cluster_name} · `}
                      {draft.dead_candidate
                        ? `${t("organizing:dead")}: ${draft.dead_reason || "—"}` : t("organizing:active")}
                    </div>
                    <div style={{ marginTop: 8, fontSize: 10.5, color: "var(--tx5)" }}>
                      {t("organizing:sources")}: {(target.sources || [])
                        .map(source => source.digest).join(" · ")}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
          {error && <div style={{ color: "var(--err-text)", fontSize: 11,
            marginTop: 12 }}>{error}</div>}
        </div>
        {proposal?.status === "pending" && (
          <div style={{ minHeight: 54, padding: "10px 18px", borderTop: "1px solid var(--line4)",
            display: "flex", gap: 8, justifyContent: "flex-end" }}>
            <button className="fbtn" disabled={busy} onClick={saveEdits}>
              {t("organizing:saveChanges")}</button>
            <button className="fbtn" disabled={busy} onClick={() => decide("reject")}>
              {t("organizing:reject")}</button>
            <button className="fbtn-primary" disabled={busy} onClick={() => decide("approve")}>
              {t("organizing:approve")}</button>
          </div>
        )}
      </div>
    </div>
  );
}
