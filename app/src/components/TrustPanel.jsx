import { TOOLS, TOOL_NAME } from "../api.js";

function TrustPanel({ env }) {
  return (
    <div className="trust">
      <div className="trust-title">环境与信任</div>
      <div className="trust-body">
        {env ? TOOLS.map(k => {
          const v = env[k] || {};
          const dot = v.installed ? (v.verified ? "ok" : "warn") : "miss";
          const ver = !v.installed ? "未安装"
            : `v${v.version || "?"}${v.verified ? " · 已验证" : ""}`;
          return (
            <div key={k}>
              <div className="trust-row">
                <span className={`dot ${dot}`} />
                <span className="name">{TOOL_NAME[k]}</span>
                <span className="ver">{ver}</span>
              </div>
              {v.installed && !v.verified &&
                <div className="trust-warn">与黄金样本 {v.golden || "—"} 不一致 · 建议自检</div>}
            </div>
          );
        }) : <span className="small muted">检测中…</span>}
      </div>
      <div className="trust-foot">版本漂移时自动降级迁移策略。</div>
    </div>
  );
}

export default TrustPanel;
