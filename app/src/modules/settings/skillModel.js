// 技能页的纯函数:候选与已导入的匹配、按来源分组、体积格式化。
// 这里是「候选 ≠ 已导入」这条边界在前端的落点——候选永远只带 candidateId,
// 只有 installedId 非空才说明库里已经有一份。

/** 候选与库里已有技能的匹配:同名目录视为同一个技能的不同版本。 */
const candidateInstalledId = (candidate, skills) => {
  const target = candidate.candidateId.split(":").slice(1).join(":");
  return skills.find(skill => skill.id === target)?.id ?? null;
};

export const decorateCandidates = (candidates, skills) =>
  candidates.map(candidate => ({
    ...candidate,
    installedId: candidateInstalledId(candidate, skills),
  }));

/** 按来源分组;来源本身没有候选也保留一组,这样"目录空的"和"目录不存在"都看得见。 */
export function groupBySource(candidates, sources) {
  return sources.map(source => ({
    source,
    items: candidates.filter(candidate => candidate.source === source.id),
  }));
}

export const isGlobal = (skillId, global) => (global || []).includes(skillId);

export function toggleGlobal(skillId, global) {
  const current = global || [];
  return current.includes(skillId)
    ? current.filter(id => id !== skillId)
    : [...current, skillId];
}

export function formatSkillSize(bytes) {
  const value = Number(bytes) || 0;
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

/** 角色挂着但库里已经没有的技能 id:UI 上单列出来让用户摘掉。 */
export const missingSkillIds = (selected, skills) => {
  const known = new Set(skills.map(skill => skill.id));
  return (selected || []).filter(id => !known.has(id));
};
