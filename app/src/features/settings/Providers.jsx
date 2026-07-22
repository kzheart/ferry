// 设置 · 提供商:左侧是已添加的 Provider,右侧配置凭据并勾选哪些模型出现在模型选择器里
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { agentCommand } from "../../api/agent/agentClient.js";
import { Spinner } from "../../components/ui/icons.jsx";
import { inputStyle, Toggle } from "./parts.jsx";

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
                style={{ ...inputStyle, flex: 1 }} />
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

// 「+」弹层:列出尚未添加的内置 Provider
function AddMenu({ candidates, onPick, onClose }) {
  const { t } = useTranslation();
  const [q, setQ] = useState("");
  const matched = candidates.filter(p =>
    !q || p.name.toLowerCase().includes(q.toLowerCase())
    || p.id.toLowerCase().includes(q.toLowerCase()));
  return (
    <>
      <div onMouseDown={onClose} style={{ position: "fixed", inset: 0, zIndex: 69 }} />
      <div style={{ position: "absolute", left: 0, bottom: "100%", marginBottom: 6, width: 236,
        background: "var(--bg)", borderRadius: 11, boxShadow: "var(--shadow-menu)",
        padding: 6, zIndex: 70, animation: "fpop .14s ease" }}>
        <input autoFocus value={q} onChange={e => setQ(e.target.value)}
          placeholder={t("settings:providers.searchProvider")}
          style={{ ...inputStyle, width: "100%", height: 28, marginBottom: 4 }} />
        <div className="fscroll" style={{ maxHeight: 260, overflowY: "auto" }}>
          {matched.map(p => (
            <div key={p.id} className="hov-item"
              onMouseDown={e => { e.preventDefault(); onPick(p); }}
              style={{ padding: "6px 8px", borderRadius: 7, fontSize: 12.5, color: "var(--tx1)",
                cursor: "default", overflow: "hidden", textOverflow: "ellipsis",
                whiteSpace: "nowrap" }}>{p.name}</div>
          ))}
          {!matched.length && (
            <div style={{ padding: "10px 8px", fontSize: 11.5, color: "var(--tx5)" }}>
              {t("settings:providers.noCandidate")}</div>)}
        </div>
      </div>
    </>
  );
}

export default function Providers({ ferry }) {
  const { t } = useTranslation();
  const [providers, setProviders] = useState(null);
  const [visible, setVisible] = useState({});
  const [selId, setSelId] = useState(null);
  const [models, setModels] = useState([]);
  const [query, setQuery] = useState("");
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState(null);
  const [adding, setAdding] = useState(false);
  const selIdRef = useRef(selId); selIdRef.current = selId;

  const load = useCallback(async () => {
    const [list, config] = await Promise.all([
      agentCommand("providers.list"), agentCommand("config.get")]);
    setProviders(list);
    setVisible(config.visible_models || {});
    return list;
  }, []);

  useEffect(() => { load().catch(e => setNotice(String(e.message || e))); }, [load]);

  const loadModels = useCallback(async providerId => {
    if (!providerId) return setModels([]);
    const list = await agentCommand("models.list",
      { provider_id: providerId, limit: 200 }).catch(() => []);
    setModels(list || []);
  }, []);
  useEffect(() => { loadModels(selId); }, [selId, loadModels]);

  // OAuth 登录完成后凭据与动态模型目录都会变
  useEffect(() => {
    if (ferry?.auth?.status !== "completed") return;
    load().catch(() => {});
    agentCommand("models.refresh")
      .then(() => loadModels(selIdRef.current))
      .catch(() => {});
    ferry.loadModels?.();
  }, [ferry?.auth?.status]);

  const enabled = useMemo(() => (providers || []).filter(p => p.enabled), [providers]);
  const candidates = useMemo(() => (providers || []).filter(p => !p.enabled), [providers]);
  const sel = enabled.find(p => p.id === selId) || null;
  useEffect(() => {
    if (!sel && enabled.length) setSelId(enabled[0].id);
  }, [sel, enabled]);

  const act = async fn => {
    setBusy(true); setNotice(null);
    try { await fn(); }
    catch (e) { setNotice(String(e.message || e)); }
    setBusy(false);
  };
  // 凭据变化会改变模型选择器的可用项,顺带刷新对话侧的健康态
  const syncFerry = async () => {
    await ferry?.loadModels?.();
    await ferry?.refresh?.();
  };

  const addProvider = p => act(async () => {
    await agentCommand("provider.enabled.set", { provider_id: p.id, enabled: true });
    await load();
    setSelId(p.id);
    setAdding(false);
  });

  const removeProvider = () => sel && act(async () => {
    if (sel.custom) await agentCommand("custom_provider.delete", { provider_id: sel.id });
    else await agentCommand("provider.enabled.set", { provider_id: sel.id, enabled: false });
    setSelId(null);
    await load();
    await syncFerry();
  });

  const saveKey = () => act(async () => {
    await agentCommand("credential.set", { provider_id: sel.id, key });
    setKey("");
    setNotice(t("settings:providers.keySaved"));
    await load();
    await syncFerry();
  });

  const logout = () => act(async () => {
    await agentCommand("provider.logout", { provider_id: sel.id });
    await load();
    await syncFerry();
  });

  const refreshCatalog = () => act(async () => {
    const r = await agentCommand("models.refresh");
    setNotice(r.failed_provider_ids?.length
      ? t("settings:providers.refreshPartial", { list: r.failed_provider_ids.join(", ") })
      : t("settings:providers.refreshDone"));
    await loadModels(selId);
    await load();
  });

  // visible_models 缺省表示全部可见;取消勾选时才写入显式白名单
  const shownIds = sel ? visible[sel.id] : undefined;
  const isShown = id => !shownIds || shownIds.includes(id);
  const toggleModel = id => act(async () => {
    const all = models.map(m => m.id);
    const base = shownIds || all;
    const next = base.includes(id) ? base.filter(x => x !== id) : [...base, id];
    const payload = next.length === all.length && all.every(x => next.includes(x)) ? null : next;
    await agentCommand("models.visibility.set", { provider_id: sel.id, model_ids: payload });
    await load();
    await ferry?.loadModels?.();
  });

  const filtered = models.filter(m => !query
    || m.id.toLowerCase().includes(query.toLowerCase())
    || m.name.toLowerCase().includes(query.toLowerCase()));

  return (
    <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
      {/* Provider 列表 */}
      <div style={{ width: 204, flex: "none", borderRight: "1px solid var(--line4)",
        display: "flex", flexDirection: "column", minHeight: 0 }}>
        <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "12px 8px" }}>
          {providers === null && (
            <div style={{ padding: 20, textAlign: "center" }}><Spinner /></div>)}
          {enabled.map(p => (
            <div key={p.id} className={p.id === selId ? undefined : "hov-item"}
              onClick={() => { setSelId(p.id); setQuery(""); setNotice(null); setKey(""); }}
              style={{ display: "flex", alignItems: "center", gap: 8, height: 32, padding: "0 10px",
                borderRadius: 8, cursor: "default",
                background: p.id === selId ? "var(--seg-on)" : "transparent" }}>
              <span style={{ fontSize: 12.5, flex: 1, minWidth: 0, overflow: "hidden",
                textOverflow: "ellipsis", whiteSpace: "nowrap",
                color: p.id === selId ? "var(--tx1)" : "var(--tx2b)",
                fontWeight: p.id === selId ? 650 : 500 }}>{p.name}</span>
              {p.configured && <span style={{ width: 6, height: 6, borderRadius: "50%",
                background: "var(--ok)", flex: "none" }} />}
            </div>
          ))}
        </div>
        <div style={{ position: "relative", flex: "none", display: "flex", gap: 2,
          padding: "6px 10px", borderTop: "1px solid var(--line4)" }}>
          {adding && (
            <AddMenu candidates={candidates} onPick={addProvider}
              onClose={() => setAdding(false)} />)}
          <button className="hov" title={t("settings:providers.add")} disabled={busy}
            onClick={() => setAdding(v => !v)}
            style={{ width: 26, height: 26, border: "none", borderRadius: 6, cursor: "default",
              background: "transparent", color: "var(--tx3b)", fontSize: 16, lineHeight: 1 }}>+</button>
          <button className="hov" title={t("settings:providers.remove")}
            disabled={!sel || busy} onClick={removeProvider}
            style={{ width: 26, height: 26, border: "none", borderRadius: 6, cursor: "default",
              background: "transparent", color: sel ? "var(--tx3b)" : "var(--tx5)",
              fontSize: 16, lineHeight: 1 }}>−</button>
        </div>
      </div>

      {/* 详情 */}
      <div className="fscroll" style={{ flex: 1, minWidth: 0, overflowY: "auto",
        padding: "18px 20px", display: "flex", flexDirection: "column", gap: 14 }}>
        {!sel && (
          <div style={{ color: "var(--tx5)", fontSize: 12.5, paddingTop: 40, textAlign: "center",
            lineHeight: 1.6, whiteSpace: "pre-line" }}>
            {t("settings:providers.emptyHint")}</div>)}
        {sel && (
          <>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ fontSize: 15, fontWeight: 650, color: "var(--tx1)" }}>{sel.name}</span>
              <span style={{ flex: 1 }} />
              <span style={{ fontSize: 11.5, color: sel.configured ? "var(--ok-deep)" : "var(--tx5)" }}>
                {sel.configured
                  ? t("settings:providers.configured", { type: sel.credential_type || "custom" })
                  : t("settings:providers.notConfigured")}</span>
            </div>

            {sel.auth_types.includes("api_key") && (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2b)" }}>
                  {t("settings:providers.apiKey")}</span>
                <div style={{ display: "flex", gap: 6 }}>
                  <input type="password" value={key} onChange={e => setKey(e.target.value)}
                    placeholder={sel.credential_type === "api_key"
                      ? t("settings:providers.keySet") : t("settings:providers.keyPlaceholder")}
                    style={{ ...inputStyle, flex: 1 }} />
                  <button className="fbtn fbtn-primary" disabled={!key || busy} onClick={saveKey}
                    style={{ height: 32 }}>{t("settings:providers.save")}</button>
                </div>
              </div>
            )}

            <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
              {sel.auth_types.includes("oauth") && !ferry?.auth && (
                <button className="fbtn" disabled={busy}
                  onClick={() => act(() => ferry.startLogin(sel.id, "oauth"))}>
                  {t("settings:providers.oauthLogin")}</button>)}
              {sel.configured && !sel.custom && (
                <button className="fbtn" disabled={busy} onClick={logout}>
                  {t("settings:providers.logout")}</button>)}
            </div>
            <AuthFlow auth={ferry?.auth} ferry={ferry} />

            {notice && (
              <div style={{ fontSize: 11.5, color: "var(--acc-text)", background: "var(--acc-soft3)",
                border: "1px solid var(--acc-line)", borderRadius: 8, padding: "7px 10px" }}>
                {notice}</div>)}

            {/* 模型可见性 */}
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 4 }}>
              <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2b)", flex: 1 }}>
                {t("settings:providers.models")}</span>
              <span style={{ fontSize: 11, color: "var(--tx5)" }}>
                {t("settings:providers.shownCount",
                  { n: sel.visible_model_count, total: sel.model_count })}</span>
              <button className="fbtn" onClick={refreshCatalog} disabled={busy}
                style={{ height: 28, fontSize: 11.5 }}>
                {t("settings:providers.refreshModels")}</button>
            </div>
            <input value={query} onChange={e => setQuery(e.target.value)}
              placeholder={t("settings:providers.searchModel")}
              style={{ ...inputStyle, width: "100%" }} />
            <div style={{ display: "flex", flexDirection: "column" }}>
              {filtered.map((m, i) => (
                <div key={m.id} style={{ display: "flex", alignItems: "center", gap: 10,
                  padding: "9px 2px", borderTop: i === 0 ? "none" : "1px solid var(--line6)" }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 12.5, color: "var(--tx1)", overflow: "hidden",
                      textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{m.name}</div>
                    <div className="mono" style={{ fontSize: 10.5, color: "var(--tx5)", marginTop: 1 }}>
                      {m.id} · {Math.round(m.context_window / 1000)}k
                      {m.input.includes("image") ? ` · ${t("settings:providers.image")}` : ""}
                      {m.reasoning ? ` · ${t("settings:providers.reasoning")}` : ""}</div>
                  </div>
                  <Toggle size={20} on={isShown(m.id)} onChange={() => toggleModel(m.id)} />
                </div>
              ))}
              {!filtered.length && (
                <div style={{ fontSize: 11.5, color: "var(--tx5)", padding: "12px 0" }}>
                  {t("settings:providers.noModels")}</div>)}
            </div>
            <div style={{ fontSize: 11, color: "var(--tx5)", lineHeight: 1.55 }}>
              {t("settings:providers.plaintextNote")}</div>
          </>
        )}
      </div>
    </div>
  );
}
