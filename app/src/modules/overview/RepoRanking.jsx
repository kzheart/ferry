import { useMemo } from "react";
import { TOOLS, TOOL_NAME } from "../../shared/contracts/tools.js";
import { rankRepos } from "./overviewModel.js";
import { Card, num, toolColor } from "./primitives.jsx";

export default function RepoRanking({ repos, metric, t, fmtTokens, fmtInt, extra }) {
  const rankedRepos = useMemo(
    () => rankRepos(repos, metric),
    [repos, metric],
  );
  const repoValue = r => (metric === "tokens" ? r.tokens : r.sessions);
  const repoByTool = r => (metric === "tokens" ? r.byToolTokens : r.byToolSessions);
  const fmtRepo = metric === "tokens" ? fmtTokens : fmtInt;
  const maxRepoValue = rankedRepos[0] ? repoValue(rankedRepos[0]) : 1;
  return (
    <Card title={t("overview:repo.title")} sub={t("overview:repo.sub")} extra={extra} fill>
      {rankedRepos.length ? (
        <div style={{ display: "flex", flexDirection: "column", flex: 1, gap: 9 }}>
          {rankedRepos.map(r => (
            <div key={r.name} style={{ display: "grid", gridTemplateColumns: "110px 1fr auto", gap: 10,
              alignItems: "center", flex: 1 }}>
              <span title={r.name} style={{ fontSize: 12, color: "var(--tx2)", whiteSpace: "nowrap",
                overflow: "hidden", textOverflow: "ellipsis" }}>{r.name}</span>
              <div style={{ height: 7, background: "var(--track)", borderRadius: 4, overflow: "hidden", display: "flex" }}>
                {TOOLS.map(tl => {
                  const w = (repoByTool(r)[tl] || 0) / maxRepoValue * 100;
                  return w ? <i key={tl} style={{ display: "block", height: "100%", width: `${w}%`, background: toolColor(tl) }} /> : null;
                })}
              </div>
              <span style={{ fontSize: 11, color: "var(--tx3)", minWidth: 52, textAlign: "right", ...num }}>{fmtRepo(repoValue(r))}</span>
            </div>
          ))}
        </div>
      ) : (
        <div style={{ padding: "24px 8px", textAlign: "center", color: "var(--tx5)", fontSize: 12, flex: 1 }}>
          {t("overview:repo.empty")}
        </div>
      )}
      <div style={{ display: "flex", gap: 13, flexWrap: "wrap", fontSize: 11, color: "var(--tx3)", marginTop: 12 }}>
        {TOOLS.map(tl => (
          <span key={tl} style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
            <i style={{ width: 8, height: 8, borderRadius: 2, background: toolColor(tl) }} />{TOOL_NAME[tl] || tl}
          </span>
        ))}
      </div>
    </Card>
  );
}
