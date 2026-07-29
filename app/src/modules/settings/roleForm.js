import { DEFAULT_ROLE_COLOR, DEFAULT_ROLE_ICON } from "../../shared/ui/roleIcons.js";

// 与 ferry-runtime 的 FERRY_TOOL_NAMES 对齐。
// 读/写不在这里区分:审批由运行时按工具语义强制,UI 上再标一遍只会让人以为是可选项。
export const TOOLS = [
  "session_search",
  "session_read",
  "usage",
  "migrate",
  "session_edit",
  "bash",
  "agent_prompt",
];

export function blankRole() {
  return {
    id: "",
    name: "",
    description: "",
    icon: DEFAULT_ROLE_ICON,
    color: DEFAULT_ROLE_COLOR,
    persona: "",
    tools: ["session_search", "session_read", "usage"],
    skills: [],
    apply_policy: "manual",
  };
}

// 详情表单只吃可编辑字段:builtin 由运行时算出,带进 draft 会在保存时被拒
export function editable(role) {
  return {
    id: role.id,
    name: role.name,
    description: role.description || "",
    icon: role.icon || DEFAULT_ROLE_ICON,
    color: role.color || DEFAULT_ROLE_COLOR,
    persona: role.persona || "",
    tools: [...(role.tools || [])],
    skills: [...(role.skills || [])],
    apply_policy: role.apply_policy || "manual",
    ...(role.model ? { model: role.model } : {}),
    ...(role.thinking ? { thinking: role.thinking } : {}),
    ...(role.optimizer === true ? { optimizer: true } : {}),
  };
}

// 提交给运行时前去掉空说明与空模型,避免写入空字符串字段
export function payload(draft) {
  const role = { ...draft, description: draft.description.trim() };
  if (!role.description) delete role.description;
  if (!role.model?.provider || !role.model?.model) delete role.model;
  return role;
}

export const modelKey = model => (model ? `${model.provider}/${model.model}` : "");
