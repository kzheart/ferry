/**
 * skill 工具:按需读取技能正文。
 * 读的是 Ferry 自己数据目录下、用户明确导入的文件,在 runtime 本地执行,不经 RuntimeGateway。
 */
import type { AgentTool } from "@earendil-works/pi-agent-core";
import { Type } from "@earendil-works/pi-ai";

export interface SkillReadResult {
  name: string;
  body: string;
  files: string[];
}

export function createSkillTool(
  read: (id: string) => Promise<SkillReadResult>,
  allowed: readonly string[],
): AgentTool {
  return {
    name: "skill",
    label: "skill",
    description:
      "Load the full instructions of one of the skills listed in the system prompt. Call this before acting on a task that matches a skill, then follow the returned instructions.",
    parameters: Type.Object(
      {
        skill_id: Type.String({
          minLength: 1,
          maxLength: 64,
          description: "Skill id exactly as listed in the system prompt.",
          ...(allowed.length > 0 ? { enum: [...allowed] } : {}),
        }),
      },
      { additionalProperties: false },
    ),
    executionMode: "parallel",
    async execute(_toolCallId, params) {
      const id = String((params as { skill_id?: unknown }).skill_id ?? "");
      if (!allowed.includes(id)) {
        throw new Error(`skill ${id} is not available in this session`);
      }
      const skill = await read(id);
      const text =
        skill.files.length > 1
          ? `${skill.body}\n\n---\nBundled files: ${skill.files.join(", ")}`
          : skill.body;
      return {
        content: [{ type: "text" as const, text }],
        details: { skill_id: id, name: skill.name, files: skill.files },
      };
    },
  };
}
