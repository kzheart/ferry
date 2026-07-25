// 技能分区外壳:左栏「我的技能」+「可导入」,右栏详情。
// 两区的边界就是产品语义的边界——上面是 Ferry 库里的技能,下面只是别人目录里的候选。
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import SkillLibrary from "./SkillLibrary.jsx";
import SkillDiscovery from "./SkillDiscovery.jsx";
import SkillDetail from "./SkillDetail.jsx";
import { decorateCandidates, isGlobal, toggleGlobal } from "./skillModel.js";

const sectionTitle = {
  fontSize: 10.5, fontWeight: 700, letterSpacing: ".05em", color: "var(--tx5)",
  padding: "10px 8px 5px",
};

function FooterButton({ label, onClick, busy }) {
  return (
    <button className="hov-item" onClick={onClick} disabled={busy}
      style={{ display: "block", width: "100%", height: 27, padding: "0 8px",
        border: "none", borderRadius: 7, background: "transparent", textAlign: "left",
        color: "var(--tx3b)", fontFamily: "inherit", fontSize: 11.5, fontWeight: 600,
        cursor: "default" }}>
      {label}</button>
  );
}

export default function Skills() {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const [selectedId, setSelectedId] = useState(null);
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);

  const { reloadSkills, loadSkillCandidates } = ferry;
  useEffect(() => {
    reloadSkills();
    loadSkillCandidates();
  }, [reloadSkills, loadSkillCandidates]);

  const installed = ferry.skills?.skills || [];
  const global = ferry.skills?.global || [];
  const sources = ferry.skillCandidates?.sources || ferry.skills?.scan_sources || [];
  const candidates = decorateCandidates(
    ferry.skillCandidates?.candidates || [], installed);

  const skill = selectedId?.startsWith("installed:")
    ? installed.find(item => item.id === selectedId.slice("installed:".length))
    : null;
  const candidate = selectedId?.startsWith("candidate:")
    ? candidates.find(item =>
      item.candidateId === selectedId.slice("candidate:".length))
    : null;

  // 正文按需拉取:列表里几十个技能不该把每份 SKILL.md 都读进内存
  useEffect(() => {
    let stale = false;
    if (!skill) { setBody(""); return undefined; }
    ferry.readSkill(skill.id)
      .then(result => { if (!stale) setBody(result?.body || ""); })
      .catch(() => { if (!stale) setBody(""); });
    return () => { stale = true; };
  }, [skill, ferry]);

  const guard = useCallback(async action => {
    setBusy(true);
    try { return await action(); } finally { setBusy(false); }
  }, []);

  const importCandidate = () => guard(async () => {
    const result = await ferry.importSkill({
      candidate_id: candidate.candidateId,
      overwrite: !!candidate.installedId,
    });
    const id = result?.skill?.id;
    if (id) setSelectedId(`installed:${id}`);
  });

  const removeSkill = () => guard(async () => {
    await ferry.deleteSkill(skill.id);
    setSelectedId(null);
  });

  const flipGlobal = () => guard(() =>
    ferry.setGlobalSkills(toggleGlobal(skill.id, global)));

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
      <div style={{ width: 210, flex: "none", borderRight: "1px solid var(--line4)",
        display: "flex", flexDirection: "column", minHeight: 0 }}>
        <div className="fscroll" style={{ flex: 1, overflowY: "auto", padding: "6px 8px" }}>
          <div style={sectionTitle}>{t("settings:skills.mine")}</div>
          <SkillLibrary skills={installed} global={global}
            selectedId={selectedId} onSelect={setSelectedId} />
          <div style={sectionTitle}>{t("settings:skills.available")}</div>
          <SkillDiscovery candidates={candidates} sources={sources}
            selectedId={selectedId} onSelect={setSelectedId}
            onRemoveSource={id => guard(() => ferry.removeSkillSource(id))} />
        </div>
        <div style={{ flex: "none", padding: "6px 10px",
          borderTop: "1px solid var(--line4)" }}>
          <FooterButton busy={busy} label={t("settings:skills.addSource")}
            onClick={() => guard(() => ferry.addSkillSource())} />
          <FooterButton busy={busy} label={t("settings:skills.importFolder")}
            onClick={() => guard(() => ferry.importSkillFolder())} />
        </div>
      </div>

      <div className="fscroll"
        style={{ flex: 1, minWidth: 0, overflowY: "auto", padding: "0 24px 24px" }}>
        <SkillDetail skill={skill} candidate={candidate} body={body} busy={busy}
          isGlobalSkill={skill ? isGlobal(skill.id, global) : false}
          onToggleGlobal={flipGlobal} onDelete={removeSkill} onImport={importCandidate} />
      </div>
    </div>
  );
}
