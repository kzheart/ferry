/** SKILL.md 的 frontmatter 解析。技能库与候选发现共用,故独立成文件。 */

export const SKILL_MANIFEST = "SKILL.md";

export interface SkillDocument {
  name: string;
  description: string;
}

const MAX_NAME = 200;
const MAX_DESCRIPTION = 500;

function clip(value: string, maximum: number) {
  const text = value.replace(/\s+/g, " ").trim();
  return text.length > maximum ? `${text.slice(0, maximum - 1)}…` : text;
}

/** 只认 --- 包围的首个块里的 name/description 两个标量键;不引 yaml 依赖。 */
function frontmatter(markdown: string): Record<string, string> {
  const lines = markdown.split(/\r?\n/);
  if (lines[0]?.trim() !== "---") return {};
  const end = lines.findIndex(
    (line, index) => index > 0 && line.trim() === "---",
  );
  if (end < 0) return {};
  const fields: Record<string, string> = {};
  for (const line of lines.slice(1, end)) {
    const match = /^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$/.exec(line);
    if (!match?.[1]) continue;
    let value = (match[2] ?? "").trim();
    // 去掉成对引号,不做转义解析——技能元信息不该复杂到需要它
    if (value.length >= 2 && (value.startsWith('"') || value.startsWith("'"))) {
      if (value.at(-1) === value[0]) value = value.slice(1, -1);
    }
    fields[match[1].toLowerCase()] = value;
  }
  return fields;
}

/** frontmatter 缺失时,name 回落目录名、description 回落正文首个非空段落。 */
export function parseSkillDocument(
  markdown: string,
  fallbackName: string,
): SkillDocument {
  const fields = frontmatter(markdown);
  const body = markdown.startsWith("---")
    ? markdown.slice(markdown.indexOf("\n---", 3) + 4)
    : markdown;
  const paragraph =
    body
      .split(/\r?\n\s*\r?\n/)
      .map((block) => block.replace(/^#+\s*/gm, "").trim())
      .find((block) => block.length > 0) ?? "";
  const name = fields.name ? clip(fields.name, MAX_NAME) : "";
  const description = fields.description
    ? clip(fields.description, MAX_DESCRIPTION)
    : clip(paragraph, MAX_DESCRIPTION);
  return { name: name || fallbackName, description };
}
