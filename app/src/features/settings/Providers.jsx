// 设置 · 提供商:左侧是已添加的 Provider,右侧只配置凭据;配好凭据它的模型就自动进入「模型」页
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { runtime } from "../../api/transport/desktopClient.js";
import { ProviderIcon, Spinner } from "../../components/ui/icons.jsx";
import { Check, inputStyle } from "./parts.jsx";

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

// 手填模型:Provider 目录里没有的新模型,填 ID 就能用,能力字段决定它能收什么输入
function AddModel({ onSubmit, onCancel, busy }) {
  const { t } = useTranslation();
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [image, setImage] = useState(false);
  const [reasoning, setReasoning] = useState(false);
  const [ctx, setCtx] = useState("");
  const submit = () => id.trim() && onSubmit({
    model_id: id.trim(),
    ...(name.trim() ? { name: name.trim() } : {}),
    image, reasoning,
    ...(Number(ctx) > 0 ? { context_window: Math.round(Number(ctx) * 1000) } : {}),
  });
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8, padding: 12,
      border: "1px solid var(--line4)", borderRadius: 10, background: "var(--surface)" }}>
      <div style={{ display: "flex", gap: 6 }}>
        <input autoFocus value={id} onChange={e => setId(e.target.value)}
          onKeyDown={e => { if (e.key === "Escape") onCancel(); }}
          placeholder={t("settings:models.idPlaceholder")}
          className="mono" style={{ ...inputStyle, flex: 2 }} />
        <input value={name} onChange={e => setName(e.target.value)}
          onKeyDown={e => { if (e.key === "Escape") onCancel(); }}
          placeholder={t("settings:models.namePlaceholder")}
          style={{ ...inputStyle, flex: 1 }} />
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 14, flexWrap: "wrap" }}>
        <Capability on={image} onChange={setImage} label={t("settings:models.image")} />
        <Capability on={reasoning} onChange={setReasoning} label={t("settings:models.reasoning")} />
        <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <span style={{ fontSize: 11.5, color: "var(--tx3b)" }}>
            {t("settings:models.contextWindow")}</span>
          <input value={ctx} onChange={e => setCtx(e.target.value.replace(/[^\d]/g, ""))}
            placeholder="128" className="mono"
            style={{ ...inputStyle, width: 62, height: 26, fontSize: 11.5 }} />
          <span style={{ fontSize: 11.5, color: "var(--tx5)" }}>k</span>
        </span>
      </div>
      <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
        <button className="fbtn" onClick={onCancel}>{t("settings:models.cancel")}</button>
        <button className="fbtn fbtn-primary" disabled={!id.trim() || busy} onClick={submit}
          style={{ height: 28 }}>{t("settings:models.addConfirm")}</button>
      </div>
    </div>
  );
}

// 能力开关:小尺寸勾选框,和模型页的选中态同一套视觉
function Capability({ on, onChange, label }) {
  return (
    <span onClick={() => onChange(!on)}
      style={{ display: "inline-flex", alignItems: "center", gap: 6, cursor: "default" }}>
      <Check on={on} size={15} />
      <span style={{ fontSize: 11.5, color: "var(--tx2b)" }}>{label}</span>
    </span>
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
              style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 8px",
                borderRadius: 7, cursor: "default" }}>
              <ProviderIcon provider={p.id} size={15} />
              <span style={{ fontSize: 12.5, color: "var(--tx1)", minWidth: 0, overflow: "hidden",
                textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{p.name}</span>
            </div>
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
  const [selId, setSelId] = useState(null);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState(null);
  const [adding, setAdding] = useState(false);
  const [catalog, setCatalog] = useState([]);
  const [addingModel, setAddingModel] = useState(false);

  const load = useCallback(async () => {
    const list = await runtime("providers.list");
    setProviders(list);
    // 模型目录里带 custom 标记,能区分手填模型与 Provider 自带的
    setCatalog(await runtime("models.catalog").catch(() => []));
    return list;
  }, []);

  // 失败也要落地成空列表,否则 providers 停在 null,左栏 spinner 会一直转
  useEffect(() => {
    load().catch(e => { setNotice(String(e.message || e)); setProviders([]); });
  }, [load]);

  // OAuth 登录完成后凭据与动态模型目录都会变
  useEffect(() => {
    if (ferry?.auth?.status !== "completed") return;
    load().catch(() => {});
    runtime("models.refresh").catch(() => {});
    ferry.loadModels?.();
  }, [ferry?.auth?.status]);

  const enabled = useMemo(() => (providers || []).filter(p => p.enabled), [providers]);
  const candidates = useMemo(() => (providers || []).filter(p => !p.enabled), [providers]);
  const sel = enabled.find(p => p.id === selId) || null;
  const selModels = useMemo(
    () => catalog.filter(m => m.provider === selId), [catalog, selId]);
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
    await runtime("provider.enabled.set", {
      provider_id: p.id,
      enabled: true,
    });
    await load();
    setSelId(p.id);
    setAdding(false);
  });

  const removeProvider = () => sel && act(async () => {
    if (sel.custom) {
      await runtime("custom_provider.delete", { provider_id: sel.id });
    } else {
      await runtime("provider.enabled.set", {
        provider_id: sel.id,
        enabled: false,
      });
    }
    setSelId(null);
    await load();
    await syncFerry();
  });

  const saveKey = () => act(async () => {
    await runtime("credential.set", { provider_id: sel.id, key });
    setKey("");
    // 有了凭据才能问 Provider 要动态模型表,存完 Key 立刻拉一次
    await runtime("models.refresh").catch(() => {});
    await load();
    await syncFerry();
  });

  const testProvider = () => act(async () => {
    const r = await runtime("provider.test", { provider_id: sel.id });
    setNotice(t("settings:providers.testOk", { model: r.model, ms: r.latency_ms }));
  });

  const addModel = payload => act(async () => {
    await runtime("custom_model.add", { provider_id: sel.id, ...payload });
    setAddingModel(false);
    await load();
    await syncFerry();
  });

  const deleteModel = modelId => act(async () => {
    await runtime("custom_model.delete", {
      provider_id: sel.id,
      model_id: modelId,
    });
    await load();
    await syncFerry();
  });

  const logout = () => act(async () => {
    await runtime("provider.logout", { provider_id: sel.id });
    await load();
    await syncFerry();
  });

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
              onClick={() => {
                setSelId(p.id); setNotice(null); setKey(""); setAddingModel(false);
              }}
              style={{ display: "flex", alignItems: "center", gap: 8, height: 32, padding: "0 10px",
                borderRadius: 8, cursor: "default",
                background: p.id === selId ? "var(--seg-on)" : "transparent" }}>
              <ProviderIcon provider={p.id} size={16} />
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
            <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
              <ProviderIcon provider={sel.id} size={20} />
              <span style={{ fontSize: 15, fontWeight: 650, color: "var(--tx1)" }}>{sel.name}</span>
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
              {/* 只有不支持 API Key 的 Provider 才走 OAuth,Anthropic 一律用 API Key */}
              {sel.auth_types.includes("oauth") && !sel.auth_types.includes("api_key")
                && !ferry?.auth && (
                <button className="fbtn" disabled={busy}
                  onClick={() => act(() => ferry.startLogin(sel.id, "oauth"))}>
                  {t("settings:providers.oauthLogin")}</button>)}
              {sel.configured && (
                <button className="fbtn" disabled={busy} onClick={testProvider}>
                  {t("settings:providers.test")}</button>)}
              {sel.configured && !sel.custom && (
                <button className="fbtn" disabled={busy} onClick={logout}>
                  {/* API Key 是「清除」,OAuth 才叫「退出登录」 */}
                  {t(sel.credential_type === "oauth"
                    ? "settings:providers.logout" : "settings:providers.clearKey")}</button>)}
            </div>
            <AuthFlow auth={ferry?.auth} ferry={ferry} />

            {notice && (
              <div style={{ fontSize: 11.5, color: "var(--acc-text)", background: "var(--acc-soft3)",
                border: "1px solid var(--acc-line)", borderRadius: 8, padding: "7px 10px" }}>
                {notice}</div>)}

            {sel.configured && (
              <div style={{ display: "flex", flexDirection: "column", marginTop: 4 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8, paddingBottom: 6 }}>
                  <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2b)" }}>
                    {t("settings:providers.models")}</span>
                  <span style={{ fontSize: 11, color: "var(--tx5)" }}>{selModels.length}</span>
                  <span style={{ flex: 1 }} />
                  <button className="fbtn" disabled={busy} style={{ height: 26, fontSize: 11 }}
                    onClick={() => setAddingModel(v => !v)}>
                    {t("settings:models.add")}</button>
                </div>
                {addingModel && (
                  <div style={{ paddingBottom: 8 }}>
                    <AddModel busy={busy} onSubmit={addModel}
                      onCancel={() => setAddingModel(false)} />
                  </div>)}
                {selModels.map((m, i) => (
                  <div key={m.id} style={{ display: "flex", alignItems: "center", gap: 10,
                    padding: "7px 2px", borderTop: i === 0 ? "none" : "1px solid var(--line6)" }}>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 12.5, color: "var(--tx1)", overflow: "hidden",
                        textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{m.name}</div>
                      <div className="mono"
                        style={{ fontSize: 10.5, color: "var(--tx5)", marginTop: 1 }}>
                        {m.id} · {Math.round(m.context_window / 1000)}k
                        {m.input.includes("image") ? ` · ${t("settings:models.image")}` : ""}
                        {m.reasoning ? ` · ${t("settings:models.reasoning")}` : ""}</div>
                    </div>
                    {m.custom && (
                      <button className="hov" disabled={busy} title={t("settings:models.delete")}
                        onClick={() => deleteModel(m.id)}
                        style={{ width: 24, height: 24, border: "none", borderRadius: 6,
                          flex: "none", background: "transparent", color: "var(--tx4)",
                          cursor: "default", display: "inline-flex", alignItems: "center",
                          justifyContent: "center" }}>
                        <svg viewBox="0 0 14 14" style={{ width: 12, height: 12 }} aria-hidden>
                          <path d="M2.6 3.9h8.8M5.6 3.9V2.7h2.8v1.2M3.7 3.9l.5 7.4h5.6l.5-7.4"
                            fill="none" stroke="currentColor" strokeWidth="1.2"
                            strokeLinecap="round" strokeLinejoin="round" />
                        </svg>
                      </button>)}
                  </div>
                ))}
                {!selModels.length && (
                  <div style={{ fontSize: 11.5, color: "var(--tx5)", padding: "6px 2px" }}>
                    {t("settings:providers.noModels")}</div>)}
              </div>)}
          </>
        )}
      </div>
    </div>
  );
}
