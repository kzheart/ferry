// 设置 · 模型:凭据配好后 Provider 的模型自动进入这里,勾选哪些出现在对话的模型选择器
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { agentCommand } from "../../api/agent/agentClient.js";
import { ProviderIcon, Spinner } from "../../components/ui/icons.jsx";
import { Check, inputStyle } from "./parts.jsx";

export default function Models({ ferry, onOpenProviders }) {
  const { t } = useTranslation();
  const [catalog, setCatalog] = useState(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState(null);

  const load = useCallback(async () => {
    const list = await agentCommand("models.catalog");
    setCatalog(list || []);
  }, []);
  // 失败也要落地成空列表,否则 catalog 停在 null,骨架 spinner 会一直转
  useEffect(() => {
    load().catch(e => { setNotice(String(e.message || e)); setCatalog([]); });
  }, [load]);

  const act = async fn => {
    setBusy(true); setNotice(null);
    try { await fn(); }
    catch (e) { setNotice(String(e.message || e)); }
    setBusy(false);
  };

  // visible_models 缺省表示全部可见:全勾选时写回 null,避免新模型上线后被旧白名单挡住
  const write = (providerId, nextIds, all) => act(async () => {
    const payload = nextIds.length === all.length ? null : nextIds;
    await agentCommand("models.visibility.set", { provider_id: providerId, model_ids: payload });
    await load();
    await ferry?.loadModels?.();
  });

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const map = new Map();
    for (const m of catalog || []) {
      if (!map.has(m.provider)) {
        map.set(m.provider, { id: m.provider, name: m.provider_name, all: [], rows: [] });
      }
      const g = map.get(m.provider);
      g.all.push(m);
      if (!q || m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q)) g.rows.push(m);
    }
    return [...map.values()].filter(g => g.rows.length);
  }, [catalog, query]);

  const shownTotal = (catalog || []).filter(m => m.shown).length;

  const refresh = () => act(async () => {
    const r = await agentCommand("models.refresh");
    setNotice(r.failed_provider_ids?.length
      ? t("settings:models.refreshPartial", { list: r.failed_provider_ids.join(", ") })
      : t("settings:models.refreshDone"));
    await load();
    await ferry?.loadModels?.();
  });

  return (
    <div className="fscroll" style={{ flex: 1, minWidth: 0, overflowY: "auto",
      padding: "18px 22px", display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <input value={query} onChange={e => setQuery(e.target.value)}
          placeholder={t("settings:models.search")}
          style={{ ...inputStyle, flex: 1 }} />
        <button className="fbtn" onClick={refresh} disabled={busy} style={{ height: 32 }}>
          {t("settings:models.refresh")}</button>
      </div>
      <div style={{ fontSize: 11.5, color: "var(--tx5)" }}>
        {t("settings:models.shownCount", { n: shownTotal, total: (catalog || []).length })}</div>

      {notice && (
        <div style={{ fontSize: 11.5, color: "var(--acc-text)", background: "var(--acc-soft3)",
          border: "1px solid var(--acc-line)", borderRadius: 8, padding: "7px 10px" }}>
          {notice}</div>)}

      {catalog === null && <div style={{ padding: 30, textAlign: "center" }}><Spinner /></div>}
      {catalog !== null && !catalog.length && (
        <div style={{ color: "var(--tx5)", fontSize: 12.5, paddingTop: 34, textAlign: "center",
          lineHeight: 1.7 }}>
          {t("settings:models.empty")}
          <div style={{ marginTop: 12 }}>
            <button className="fbtn" onClick={onOpenProviders}>
              {t("settings:models.goProviders")}</button>
          </div>
        </div>)}

      {groups.map(g => {
        const shownIds = g.all.filter(m => m.shown).map(m => m.id);
        const allOn = shownIds.length === g.all.length;
        return (
          <div key={g.id} style={{ display: "flex", flexDirection: "column" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "10px 2px 8px" }}>
              <ProviderIcon provider={g.id} size={16} />
              <span style={{ fontSize: 12.5, fontWeight: 650, color: "var(--tx1)" }}>{g.name}</span>
              <span style={{ fontSize: 11, color: "var(--tx5)" }}>
                {shownIds.length}/{g.all.length}</span>
              <span style={{ flex: 1 }} />
              <button className="fbtn" disabled={busy} style={{ height: 26, fontSize: 11 }}
                onClick={() => write(g.id, allOn ? [] : g.all.map(m => m.id), g.all)}>
                {t(allOn ? "settings:models.clearAll" : "settings:models.selectAll")}</button>
            </div>
            {g.rows.map((m, i) => (
              <div key={m.id} className="hov-item" onClick={() => write(g.id,
                m.shown ? shownIds.filter(x => x !== m.id) : [...shownIds, m.id], g.all)}
                style={{ display: "flex", alignItems: "center", gap: 10, cursor: "default",
                  padding: "8px 8px", borderRadius: 8,
                  borderTop: i === 0 ? "none" : "1px solid var(--line6)" }}>
                <Check on={m.shown} />
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12.5, color: "var(--tx1)", overflow: "hidden",
                    textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{m.name}</div>
                  <div className="mono" style={{ fontSize: 10.5, color: "var(--tx5)", marginTop: 1 }}>
                    {m.id} · {Math.round(m.context_window / 1000)}k
                    {m.input.includes("image") ? ` · ${t("settings:models.image")}` : ""}
                    {m.reasoning ? ` · ${t("settings:models.reasoning")}` : ""}</div>
                </div>
                {m.custom && (
                  <span style={{ fontSize: 10, color: "var(--tx5)", flex: "none",
                    border: "1px solid var(--line4)", borderRadius: 5, padding: "1px 5px" }}>
                    {t("settings:models.customTag")}</span>)}
              </div>
            ))}
          </div>
        );
      })}
    </div>
  );
}
