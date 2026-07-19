import { TOOLS, TOOL_NAME } from "../api.js";
import Badge from "../components/Badge.jsx";

function First({ env, onScan }) {
  return (
    <div className="page"><div className="first">
      <div className="logo" style={{ width: 44, height: 44, fontSize: 20, borderRadius: 11 }}>S</div>
      <div className="big">尚未扫描到任何会话</div>
      <div className="desc">Ferry 会读取本机 CLI 工具的会话记录,统一浏览、迁移与编辑。
        所有读取与处理都在本地完成,不上传任何会话内容。</div>
      <div className="tools">
        {TOOLS.map(k => {
          const v = (env || {})[k] || {};
          return (
            <div className="card tool-card" key={k}>
              <div style={{ display: "flex", justifyContent: "center", marginBottom: 8 }}><Badge tool={k} /></div>
              <div style={{ fontWeight: 600, fontSize: 13 }}>{TOOL_NAME[k]}</div>
              <div className="small muted" style={{ marginTop: 4 }}>
                {v.installed ? `已安装 · v${v.version || "?"}` : "未检测到安装"}</div>
              <div style={{ marginTop: 6 }}>
                {v.installed ? <span className="tag ok">可扫描</span>
                  : <span className="small muted">跳过</span>}</div>
            </div>
          );
        })}
      </div>
      <button className="btn primary" style={{ padding: "11px 26px", fontSize: 14 }}
        onClick={onScan}>开始扫描</button>
      <div className="hint">迁移或还原后的<b>探针验收</b>会真实加载会话并进行一次极小的模型调用,
        仅用于验证可用性。</div>
    </div></div>
  );
}

export default First;
