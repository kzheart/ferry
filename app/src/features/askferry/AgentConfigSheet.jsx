// Ask Ferry 配置:Provider 凭据(API Key / OAuth 交互流程)与模型选择
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { agentCommand } from "../../api/agent/agentClient.js";
import { Sheet } from "../../components/ui/primitives.jsx";
import { Spinner } from "../../components/ui/icons.jsx";

const row = on => ({
  display: "flex", alignItems: "center", gap: 8, padding: "6px 10px", borderRadius: 7,
  cursor: "default", background: on ? "var(--acc-soft2)" : "transparent",
});

// 从 auth.event 通知里挑出可打开的授权 URL / device code
const noticeBits = notice => {
  const bits = [];
  const walk = value => {
    if (typeof value === "string") {
      if (/^https?:\/\//.test(value)) bits.push({ url: value });
    } else if (value && typeof value === "object") {
      Object.entries(value).forEach(([k, v]) => {
        if (typeof v === "string" && /code/i.test(k) && v.length < 32) bits.push({ code: v });
        else walk(v);
      });
    }
  };
  walk(notice);
  return bits;
};

function AuthFlow({ auth, ferry }) {
  const { t } = useTranslation();
  const [values, setValues] = useState({});
  if (!auth) return null;
  const done = auth.status && auth.status !== "running";
  return (
    <div style={{ border: "1px solid var(--acc-line)", background: "var(--acc-soft3)",
      borderRadius: 10, padding: 12, display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, fontWeight: 650,
        color: "var(--acc-text)" }}>
        {!done && <Spinner size={12} />}
        <span style={{ flex: 1 }}>
          {done ? t(`askferry:auth.${auth.status}`) : t("askferry:auth.inProgress")}
        </span>
        <button className="fbtn" onClick={done ? ferry.clearAuth : ferry.cancelLogin}>
          {done ? t("askferry:auth.close") : t("askferry:auth.cancel")}</button>
      </div>
      {auth.message && <div style={{ fontSize: 11.5, color: "var(--err-text)" }}>{auth.message}</div>}
      {auth.notices.flatMap((n, i) => noticeBits(n).map((bit, j) => bit.url ? (
        <a key={`${i}-${j}`} href={bit.url} target="_blank" rel="noreferrer"
          style={{ fontSize: 12, color: "var(--acc-text)", textDecoration: "underline",
            overflowWrap: "anywhere" }}>{bit.url}</a>
      ) : (
        <code key={`${i}-${j}`} className="mono selectable" style={{ fontSize: 14, fontWeight: 700,
          letterSpacing: ".12em", color: "var(--tx1)" }}>{bit.code}</code>
      )))}
      {auth.prompts.map(p => (
        <div key={p.promptId} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <span style={{ fontSize: 12, color: "var(--tx2)" }}>{p.message}</span>
          {p.type === "select" ? (
            <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
              {(p.options || []).map(o => (
                <button key={o.id} className="fbtn" style={{ justifyContent: "flex-start" }}
                  onClick={() => ferry.respondLogin(p.promptId, o.id)}>
                  {o.label}{o.description ? ` — ${o.description}` : ""}</button>
              ))}
            </div>
          ) : (
            <div style={{ display: "flex", gap: 6 }}>
              <input className="finput" type={p.type === "secret" ? "password" : "text"}
                placeholder={p.placeholder || ""}
                value={values[p.promptId] || ""}
                onChange={e => setValues(v => ({ ...v, [p.promptId]: e.target.value }))}
                onKeyDown={e => { if (e.key === "Enter" && values[p.promptId]) {
                  ferry.respondLogin(p.promptId, values[p.promptId]); } }}
                style={{ flex: 1, height: 28, border: "1px solid var(--line2)", borderRadius: 7,
                  padding: "0 9px", fontSize: 12, background: "var(--bg)", color: "var(--tx1)" }} />
              <button className="fbtn fbtn-primary" disabled={!values[p.promptId]}
                onClick={() => ferry.respondLogin(p.promptId, values[p.promptId])}>
                {t("askferry:auth.submit")}</button>
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

export default function AgentConfigSheet({ ferry, onClose }) {
  const { t } = useTranslation();
  const [providers, setProviders] = useState(null);
  const [sel, setSel] = useState(null);
  const [models, setModels] = useState([]);
  const [query, setQuery] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState(null);
  const [pq, setPq] = useState("");
  const selRef = useRef(null); selRef.current = sel;

  const loadProviders = () => agentCommand("providers.list")
    .then(setProviders).catch(e => setNotice(String(e.message || e)));
  useEffect(() => { loadProviders(); }, []);
  // 登录完成后刷新配置态与动态模型目录
  useEffect(() => {
    if (ferry.auth?.status === "completed") {
      loadProviders();
      agentCommand("models.refresh").then(() => {
        const p = selRef.current;
        if (p) agentCommand("models.list", { provider_id: p.id, limit: 200 })
          .then(setModels).catch(() => {});
      }).catch(() => {});
    }
  }, [ferry.auth?.status]);

  useEffect(() => {
    if (!sel) return;
    agentCommand("models.list", { provider_id: sel.id, query, limit: 200 })
      .then(setModels).catch(() => setModels([]));
  }, [sel?.id, query]);

  const filtered = useMemo(() => (providers || []).filter(p =>
    !pq || p.name.toLowerCase().includes(pq.toLowerCase()) ||
    p.id.toLowerCase().includes(pq.toLowerCase())), [providers, pq]);

  const act = async fn => {
    setBusy(true); setNotice(null);
    try { await fn(); }
    catch (e) { setNotice(String(e.message || e)); }
    setBusy(false);
  };

  const saveKey = () => act(async () => {
    await agentCommand("credential.set", { provider_id: sel.id, key });
    setKey("");
    setNotice(t("askferry:config.keySaved"));
    await loadProviders();
    await ferry.refresh();
  });

  const logout = () => act(async () => {
    await agentCommand("provider.logout", { provider_id: sel.id });
    await loadProviders();
    await ferry.refresh();
  });

  const pick = (m, forSession) => act(async () => {
    await ferry.selectModel(m.provider, m.id, forSession);
    setNotice(forSession ? t("askferry:config.modelSetSession", { model: m.id })
      : t("askferry:config.modelSetDefault", { model: m.id }));
  });

  const refreshCatalog = () => act(async () => {
    const r = await agentCommand("models.refresh");
    setNotice(r.failed_provider_ids?.length
      ? t("askferry:config.refreshPartial", { list: r.failed_provider_ids.join(", ") })
      : t("askferry:config.refreshDone"));
    if (sel) setModels(await agentCommand("models.list", { provider_id: sel.id, limit: 200 }));
  });

  const inputStyle = { height: 28, border: "1px solid var(--line2)", borderRadius: 7,
    padding: "0 9px", fontSize: 12, background: "var(--bg)", color: "var(--tx1)" };

  return (
    <Sheet width={780} maxHeight={620} onClose={onClose}>
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "14px 16px 10px",
        borderBottom: "1px solid var(--line5)" }}>
        <span style={{ fontSize: 14, fontWeight: 700, color: "var(--tx1)", flex: 1 }}>
          {t("askferry:config.title")}</span>
        <button className="fbtn" onClick={refreshCatalog} disabled={busy}>
          {t("askferry:config.refreshModels")}</button>
        <button className="fbtn" onClick={onClose}>{t("askferry:config.close")}</button>
      </div>
      <div style={{ display: "flex", minHeight: 0, flex: 1 }}>
        {/* Provider 列表 */}
        <div style={{ width: 240, flex: "none", borderRight: "1px solid var(--line5)",
          display: "flex", flexDirection: "column", minHeight: 0 }}>
          <div style={{ padding: "10px 10px 6px" }}>
            <input value={pq} onChange={e => setPq(e.target.value)}
              placeholder={t("askferry:config.searchProvider")}
              style={{ ...inputStyle, width: "100%" }} />
          </div>
          <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "0 8px 10px" }}>
            {providers === null && <div style={{ padding: 20, textAlign: "center" }}><Spinner /></div>}
            {filtered.map(p => (
              <div key={p.id} className="hov-item" onClick={() => { setSel(p); setQuery(""); setNotice(null); }}
                style={row(sel?.id === p.id)}>
                <span style={{ fontSize: 12, color: "var(--tx1)", flex: 1, minWidth: 0,
                  overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{p.name}</span>
                {p.configured && <span style={{ width: 6, height: 6, borderRadius: "50%",
                  background: "var(--ok)", flex: "none" }} />}
                <span style={{ fontSize: 10, color: "var(--tx5)", flex: "none" }}>{p.model_count}</span>
              </div>
            ))}
          </div>
        </div>
        {/* 详情 */}
        <div className="fscroll" style={{ flex: 1, minWidth: 0, overflowY: "auto",
          padding: "12px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
          {!sel && (
            <div style={{ color: "var(--tx5)", fontSize: 12, paddingTop: 40, textAlign: "center" }}>
              {t("askferry:config.pickProvider")}</div>)}
          {sel && (
            <>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontSize: 13, fontWeight: 700, color: "var(--tx1)" }}>{sel.name}</span>
                <span className="mono" style={{ fontSize: 10.5, color: "var(--tx5)" }}>{sel.id}</span>
                <span style={{ flex: 1 }} />
                {sel.configured
                  ? <span style={{ fontSize: 11, color: "var(--ok-deep)" }}>
                      {t("askferry:config.configured", { type: sel.credential_type || "" })}</span>
                  : <span style={{ fontSize: 11, color: "var(--tx5)" }}>
                      {t("askferry:config.notConfigured")}</span>}
              </div>
              {/* 凭据 */}
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {sel.auth_types.includes("api_key") && (
                  <div style={{ display: "flex", gap: 6 }}>
                    <input type="password" value={key} onChange={e => setKey(e.target.value)}
                      placeholder={t("askferry:config.keyPlaceholder")}
                      style={{ ...inputStyle, flex: 1 }} />
                    <button className="fbtn fbtn-primary" disabled={!key || busy} onClick={saveKey}>
                      {t("askferry:config.saveKey")}</button>
                  </div>
                )}
                <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                  {sel.auth_types.includes("oauth") && !ferry.auth && (
                    <button className="fbtn" disabled={busy}
                      onClick={() => act(() => ferry.startLogin(sel.id, "oauth"))}>
                      {t("askferry:config.oauthLogin")}</button>
                  )}
                  {sel.configured && !sel.custom && (
                    <button className="fbtn" disabled={busy} onClick={logout}>
                      {t("askferry:config.logout")}</button>
                  )}
                  <span style={{ flex: 1 }} />
                </div>
                <AuthFlow auth={ferry.auth} ferry={ferry} />
                <div style={{ fontSize: 10.5, color: "var(--tx5)", lineHeight: 1.5 }}>
                  {t("askferry:config.plaintextNote")}</div>
              </div>
              {notice && (
                <div style={{ fontSize: 11.5, color: "var(--acc-text)", background: "var(--acc-soft3)",
                  border: "1px solid var(--acc-line)", borderRadius: 8, padding: "6px 10px" }}>
                  {notice}</div>)}
              {/* 模型 */}
              <input value={query} onChange={e => setQuery(e.target.value)}
                placeholder={t("askferry:config.searchModel")}
                style={{ ...inputStyle, width: "100%" }} />
              <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                {models.map(m => (
                  <div key={m.id} style={{ display: "flex", alignItems: "center", gap: 8,
                    padding: "6px 8px", border: "1px solid var(--line5)", borderRadius: 8 }}>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 12, color: "var(--tx1)", overflow: "hidden",
                        textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{m.name}</div>
                      <div className="mono" style={{ fontSize: 10, color: "var(--tx5)" }}>
                        {m.id} · {Math.round(m.context_window / 1000)}k
                        {m.input.includes("image") ? ` · ${t("askferry:config.image")}` : ""}
                        {m.reasoning ? ` · ${t("askferry:config.reasoning")}` : ""}</div>
                    </div>
                    <button className="fbtn" disabled={busy} onClick={() => pick(m, false)}>
                      {t("askferry:config.setDefault")}</button>
                    {ferry.activeId && (
                      <button className="fbtn" disabled={busy} onClick={() => pick(m, true)}>
                        {t("askferry:config.useForChat")}</button>)}
                  </div>
                ))}
                {sel && !models.length && (
                  <div style={{ fontSize: 11.5, color: "var(--tx5)", padding: "10px 0" }}>
                    {t("askferry:config.noModels")}</div>)}
              </div>
            </>
          )}
        </div>
      </div>
    </Sheet>
  );
}
