// 技能管理:已导入技能与外部候选分成两份状态,候选每次进技能页才扫,
// 不跟着 roles 一起刷。从 useAskFerry 拆出,状态与操作都收在这里。
import { useCallback, useState } from "react";
import { pickSkillDirectory, runtime } from "../../platform/desktop/client.js";

export function useAskFerrySkills() {
  const [skills, setSkills] = useState(
    { skills: [], global: [], scan_sources: [] });
  const [skillCandidates, setSkillCandidates] = useState(
    { candidates: [], sources: [] });

  const reloadSkills = useCallback(async () => {
    const listing = await runtime("skills.list").catch(() => null);
    const next = listing || { skills: [], global: [], scan_sources: [] };
    setSkills(next);
    return next;
  }, []);
  const loadSkillCandidates = useCallback(async () => {
    const found = await runtime("skills.candidates").catch(() => null);
    const next = found || { candidates: [], sources: [] };
    setSkillCandidates(next);
    return next;
  }, []);
  const importSkill = useCallback(async params => {
    const result = await runtime("skill.import", params);
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  const deleteSkill = useCallback(async skillId => {
    const result = await runtime("skill.delete", { skill_id: skillId });
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  const setGlobalSkills = useCallback(async skillIds => {
    const result = await runtime("skills.global.set", { skill_ids: skillIds });
    await reloadSkills();
    return result;
  }, [reloadSkills]);
  const addSkillSource = useCallback(async () => {
    const path = await pickSkillDirectory();
    if (!path) return null;
    const result = await runtime("skill.source.add", { path });
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  const removeSkillSource = useCallback(async sourceId => {
    const result = await runtime("skill.source.remove", { source_id: sourceId });
    await reloadSkills();
    await loadSkillCandidates();
    return result;
  }, [reloadSkills, loadSkillCandidates]);
  /** 从文件夹直接导入:路径由系统对话框产生,webview 不能凭空指定。 */
  const importSkillFolder = useCallback(async () => {
    const path = await pickSkillDirectory();
    if (!path) return null;
    return importSkill({ path });
  }, [importSkill]);
  const readSkill = useCallback(
    skillId => runtime("skill.read", { skill_id: skillId }), []);

  return { skills, skillCandidates, reloadSkills, loadSkillCandidates,
    importSkill, importSkillFolder, deleteSkill, setGlobalSkills,
    addSkillSource, removeSkillSource, readSkill };
}
