// 探针模型选择器:目标 CLI 的模型目录 + 手填自定义 id。
import { useState } from "react";

export default function ProbeModelPicker({ catalog, loading, err, selected, custom, onSelect, onCustom, t }) {
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
