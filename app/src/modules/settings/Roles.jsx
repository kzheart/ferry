import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

const TOOLS = ["session_search", "session_read", "usage", "migrate", "session_edit"];

function blankRole(index) {
  return {
    id: `role-${index}`,
    name: "",
    description: "",
    persona: "",
    tools: ["session_search", "session_read", "usage"],
    allow_bash: false,
    apply_policy: "manual",
    thinking: "medium",
  };
}

function editable(role) {
  return {
    id: role.id,
    name: role.name,
    description: role.description || "",
    persona: role.persona || "",
    tools: [...(role.tools || [])],
    allow_bash: false,
    apply_policy: role.apply_policy || "manual",
    ...(role.model ? { model: role.model } : {}),
    ...(role.thinking ? { thinking: role.thinking } : {}),
  };
}

export default function Roles({ ferry }) {
  const { t } = useTranslation();
  const [selectedId, setSelectedId] = useState("default");
  const [draft, setDraft] = useState(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const selected = useMemo(
    () => ferry.roles.find(role => role.id === selectedId) || ferry.roles[0],
    [ferry.roles, selectedId],
  );

  useEffect(() => {
    ferry.reloadRoles().catch(error2 => setError(String(error2)));
  }, []);
  useEffect(() => {
    if (!creating && selected) setDraft(editable(selected));
  }, [creating, selected]);

  const mutate = async action => {
    setBusy(true); setError("");
    try { await action(); }
    catch (error2) { setError(error2.message || String(error2)); }
    finally { setBusy(false); }
  };
  const save = () => mutate(async () => {
    const provider = draft.model?.provider?.trim();
    const modelId = draft.model?.model?.trim();
    if ((provider && !modelId) || (!provider && modelId)) {
      throw new Error(t("settings:roles.modelPairRequired"));
    }
    const role = {
      ...draft,
      ...(draft.description.trim()
        ? { description: draft.description.trim() }
        : {}),
      ...(provider && modelId
        ? { model: { provider, model: modelId } }
        : {}),
    };
    if (!draft.description.trim()) delete role.description;
    if (!provider && !modelId) delete role.model;
    if (creating) await ferry.createRole(role);
    else await ferry.updateRole(role);
    setSelectedId(draft.id);
    setCreating(false);
  });
  const startCreate = () => {
    const next = blankRole(Date.now().toString(36));
    setDraft(next); setCreating(true); setError("");
  };
  const copy = () => mutate(async () => {
    const nextId = `${selected.id}-copy-${Date.now().toString(36)}`;
    const result = await ferry.copyRole(
      selected.id, nextId, `${selected.name} ${t("settings:roles.copySuffix")}`,
    );
    setSelectedId(result.id);
  });
  const remove = () => mutate(async () => {
    await ferry.deleteRole(selected.id);
    setSelectedId("default");
  });

  if (!draft) {
    return (
      <div style={{ flex: 1, display: "grid", placeItems: "center", padding: 32 }}>
        <div style={{ maxWidth: 340, textAlign: "center" }}>
          <div style={{ fontSize: 13, fontWeight: 650, color: "var(--tx2)" }}>
            {t("settings:roles.unavailable")}
          </div>
          <div style={{ marginTop: 6, fontSize: 11.5, lineHeight: 1.6,
            color: "var(--tx5)" }}>
            {error || t("settings:roles.unavailableDesc")}
          </div>
          <button className="fbtn" style={{ marginTop: 14 }}
            onClick={() => ferry.reloadRoles().catch(error2 => setError(String(error2)))}>
            {t("settings:roles.retry")}
          </button>
        </div>
      </div>
    );
  }
  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
      <div className="fscroll" style={{ width: 220, overflowY: "auto",
        borderRight: "1px solid var(--line4)", padding: 12 }}>
        <button className="fbtn-primary" style={{ width: "100%", height: 32 }}
          onClick={startCreate}>{t("settings:roles.create")}</button>
        <div style={{ display: "flex", flexDirection: "column", gap: 3, marginTop: 10 }}>
          {ferry.roles.map(role => (
            <button key={role.id} className="hov-item"
              onClick={() => { setSelectedId(role.id); setCreating(false); }}
              style={{ border: "none", borderRadius: 8, padding: "8px 9px",
                background: !creating && selected?.id === role.id
                  ? "var(--seg-on)" : "transparent", textAlign: "left",
                color: "var(--tx1)", fontFamily: "inherit" }}>
              <span style={{ display: "block", fontSize: 12.5, fontWeight: 650 }}>
                {role.name}</span>
              <span style={{ display: "block", marginTop: 2, fontSize: 10.5,
                color: "var(--tx5)" }}>
                {role.builtin ? t("settings:roles.builtin") :
                  t("settings:roles.toolCount", { n: role.tools.length })}
              </span>
            </button>
          ))}
        </div>
      </div>
      <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "18px 22px" }}>
        <div style={{ display: "grid", gap: 12, maxWidth: 560 }}>
          <label style={{ fontSize: 11, color: "var(--tx4)" }}>
            {t("settings:roles.id")}
            <input className="finput" value={draft.id}
              disabled={!creating}
              onChange={event => setDraft(value => ({ ...value, id: event.target.value }))}
              style={{ width: "100%", marginTop: 5 }} />
          </label>
          <label style={{ fontSize: 11, color: "var(--tx4)" }}>
            {t("settings:roles.name")}
            <input className="finput" value={draft.name}
              disabled={selected?.builtin && !creating}
              onChange={event => setDraft(value => ({ ...value, name: event.target.value }))}
              style={{ width: "100%", marginTop: 5 }} />
          </label>
          <label style={{ fontSize: 11, color: "var(--tx4)" }}>
            {t("settings:roles.description")}
            <input className="finput" value={draft.description}
              disabled={selected?.builtin && !creating}
              onChange={event => setDraft(value => ({ ...value, description: event.target.value }))}
              style={{ width: "100%", marginTop: 5 }} />
          </label>
          <label style={{ fontSize: 11, color: "var(--tx4)" }}>
            {t("settings:roles.persona")}
            <textarea className="finput" value={draft.persona} rows={6}
              disabled={selected?.builtin && !creating}
              onChange={event => setDraft(value => ({ ...value, persona: event.target.value }))}
              style={{ width: "100%", marginTop: 5, resize: "vertical" }} />
          </label>
          <fieldset disabled={selected?.builtin && !creating}
            style={{ border: "none", padding: 0, margin: 0 }}>
            <legend style={{ fontSize: 11, color: "var(--tx4)", marginBottom: 6 }}>
              {t("settings:roles.tools")}</legend>
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              {TOOLS.map(tool => (
                <label key={tool} style={{ fontSize: 11.5, color: "var(--tx2)",
                  border: "1px solid var(--line4)", borderRadius: 7, padding: "5px 7px" }}>
                  <input type="checkbox" checked={draft.tools.includes(tool)}
                    onChange={event => setDraft(value => ({
                      ...value,
                      tools: event.target.checked
                        ? [...value.tools, tool]
                        : value.tools.filter(item => item !== tool),
                    }))} />{" "}{tool}
                </label>
              ))}
            </div>
          </fieldset>
          <fieldset disabled={selected?.builtin && !creating}
            style={{ border: "none", padding: 0, margin: 0 }}>
            <legend style={{ fontSize: 11, color: "var(--tx4)", marginBottom: 6 }}>
              {t("settings:roles.defaultModel")}</legend>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
              <input className="finput" value={draft.model?.provider || ""}
                placeholder={t("settings:roles.provider")}
                onChange={event => setDraft(value => ({
                  ...value,
                  model: { ...(value.model || {}), provider: event.target.value },
                }))} />
              <input className="finput" value={draft.model?.model || ""}
                placeholder={t("settings:roles.model")}
                onChange={event => setDraft(value => ({
                  ...value,
                  model: { ...(value.model || {}), model: event.target.value },
                }))} />
            </div>
            <div style={{ marginTop: 5, fontSize: 10.5, color: "var(--tx5)" }}>
              {t("settings:roles.modelOptional")}
            </div>
          </fieldset>
          <label style={{ fontSize: 11, color: "var(--tx4)" }}>
            {t("settings:roles.thinking")}
            <select className="finput" value={draft.thinking || ""}
              disabled={selected?.builtin && !creating}
              onChange={event => setDraft(value => {
                const next = { ...value };
                if (event.target.value) next.thinking = event.target.value;
                else delete next.thinking;
                return next;
              })}
              style={{ display: "block", marginTop: 5 }}>
              <option value="">{t("settings:roles.followModel")}</option>
              <option value="off">{t("settings:roles.thinkingOff")}</option>
              <option value="low">{t("settings:roles.thinkingLow")}</option>
              <option value="medium">{t("settings:roles.thinkingMedium")}</option>
              <option value="high">{t("settings:roles.thinkingHigh")}</option>
            </select>
          </label>
          <label style={{ fontSize: 11, color: "var(--tx5)" }}>
            <input type="checkbox" checked={false} disabled />{" "}
            {t("settings:roles.bashLater")}
          </label>
          <label style={{ fontSize: 11, color: "var(--tx4)" }}>
            {t("settings:roles.applyPolicy")}
            <select className="finput" value={draft.apply_policy}
              disabled={selected?.builtin && !creating}
              onChange={event => setDraft(value => ({
                ...value, apply_policy: event.target.value,
              }))}
              style={{ display: "block", marginTop: 5 }}>
              <option value="manual">{t("settings:roles.manual")}</option>
              <option value="auto">{t("settings:roles.auto")}</option>
            </select>
          </label>
          <div style={{ fontSize: 11, color: "var(--tx5)" }}>
            {t("settings:roles.safetyNote")}
          </div>
          {error && <div style={{ color: "var(--err-text)", fontSize: 11 }}>{error}</div>}
          <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
            {!creating && (
              <button className="fbtn" disabled={busy} onClick={copy}>
                {t("settings:roles.copy")}</button>
            )}
            {!creating && !selected?.builtin && (
              <button className="fbtn" disabled={busy} onClick={remove}>
                {t("settings:roles.delete")}</button>
            )}
            {creating && (
              <button className="fbtn" disabled={busy}
                onClick={() => { setCreating(false); setDraft(editable(selected)); }}>
                {t("settings:roles.cancel")}</button>
            )}
            {(!selected?.builtin || creating) && (
              <button className="fbtn-primary" disabled={busy || !draft.name || !draft.id}
                onClick={save}>{t("settings:roles.save")}</button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
