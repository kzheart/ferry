// 自定义提供商表单:名称、API 格式、Base URL;改完自动保存,模型列表随后从端点自动拉取
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { inputStyle } from "./parts.jsx";

export function CustomProviderSettings({ sel, onSave }) {
  const { t } = useTranslation();
  const [name, setName] = useState(sel.name);
  const [api, setApi] = useState(sel.api || "openai-completions");
  const [baseUrl, setBaseUrl] = useState(sel.base_url || "");
  useEffect(() => {
    setName(sel.name);
    setApi(sel.api || "openai-completions");
    setBaseUrl(sel.base_url || "");
  }, [sel.id]);

  // 用户常漏写协议头(cpa.example.com/v1):按 https 补全,而不是静默不保存。
  const normalizedUrl = normalizeBaseUrl(baseUrl);
  const urlInvalid = baseUrl.trim() !== "" && !normalizedUrl;
  useEffect(() => {
    const dirty = name !== sel.name || api !== (sel.api || "openai-completions")
      || normalizedUrl !== (sel.base_url || "");
    if (!dirty || !name.trim() || !normalizedUrl) return undefined;
    const timer = setTimeout(() => onSave({
      provider_id: sel.id,
      name: name.trim(),
      api,
      base_url: normalizedUrl,
      models: [],
    }), 800);
    return () => clearTimeout(timer);
  }, [name, api, normalizedUrl]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", gap: 6 }}>
        <div style={{ flex: 1, display: "flex", flexDirection: "column", gap: 5 }}>
          <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2b)" }}>
            {t("settings:providers.custom.name")}</span>
          <input value={name} onChange={e => setName(e.target.value)}
            placeholder={t("settings:providers.custom.namePlaceholder")}
            style={{ ...inputStyle, width: "100%" }} />
        </div>
        <div style={{ flex: "none", display: "flex", flexDirection: "column", gap: 5 }}>
          <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2b)" }}>
            {t("settings:providers.custom.format")}</span>
          <div style={{ display: "flex", gap: 6 }}>
            {[["openai-completions", t("settings:providers.custom.formatOpenAI")],
              ["anthropic-messages", t("settings:providers.custom.formatAnthropic")]]
              .map(([value, label]) => (
                <button key={value} className="fbtn" onClick={() => setApi(value)}
                  style={{ height: 32, fontSize: 12,
                    ...(api === value ? { borderColor: "var(--accent)", color: "var(--acc-text)",
                      background: "var(--acc-soft3)", fontWeight: 600 } : {}) }}>
                  {label}</button>
              ))}
          </div>
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 5 }}>
        <span style={{ fontSize: 12, fontWeight: 600, color: "var(--tx2b)" }}>Base URL</span>
        <input value={baseUrl} onChange={e => setBaseUrl(e.target.value)}
          placeholder={api === "anthropic-messages"
            ? "https://api.example.com/anthropic" : "https://api.example.com/v1"}
          className="mono" style={{ ...inputStyle, width: "100%" }} />
        {urlInvalid && (
          <span style={{ fontSize: 11, color: "var(--danger, #c0392b)" }}>
            {t("settings:providers.custom.urlInvalid")}</span>
        )}
      </div>
    </div>
  );
}

// 补全协议头并校验;不合法返回 null。
export function normalizeBaseUrl(raw) {
  const value = raw.trim().replace(/\/+$/, "");
  if (!value) return null;
  const withScheme = /^https?:\/\//i.test(value) ? value : `https://${value}`;
  if (!/^https?:\/\/[^\s/]+(\/\S*)?$/.test(withScheme)) return null;
  return withScheme;
}
