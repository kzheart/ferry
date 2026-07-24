// 角色详情表单的取值规则:草稿字段、默认值与提交前的清理。
import { DEFAULT_ROLE_COLOR, DEFAULT_ROLE_ICON } from "../../shared/ui/roleIcons.js";

// 与 ferry-runtime 的 FERRY_TOOL_NAMES 对齐;write 决定是否按"写操作"提示
export const TOOLS = [
  { name: "session_search", write: false },
  { name: "session_read", write: false },
  { name: "usage", write: false },
  { name: "migrate", write: true },
  { name: "session_edit", write: true },
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
    allow_bash: false,
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
    allow_bash: false,
    apply_policy: role.apply_policy || "manual",
    ...(role.model ? { model: role.model } : {}),
    ...(role.thinking ? { thinking: role.thinking } : {}),
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
