import {
  prefixMode,
  type TitleLanguage,
  type TitleStyle,
} from "./title-style.js";

/** title.generate 的输入证据,字段与引擎 title_evidence 的 sessions 项一致。 */
export interface TitleEvidence {
  tool: string;
  ref: string;
  session_id?: string | null;
  revision?: number | string | null;
  title?: string | null;
  title_source?: string | null;
  project?: string | null;
  message_count?: number | null;
  turn_count?: number | null;
  user_messages?: string[];
  last_assistant_message?: string | null;
  files?: string[];
}

const LANGUAGE_RULE: Record<TitleLanguage, string> = {
  follow: "标题语言跟随该会话里用户提问所用的语言。",
  zh: "所有标题一律用简体中文。",
  en: "所有标题一律用英文。",
};

const PRESET_RULE: Record<TitleStyle["preset"], string> = {
  ferry: "预设「ferry」:正文是动词开头的陈述短句,不要疑问句、不要标点收尾。",
  english:
    "预设「english」:正文是全小写英文短标签(可用空格或连字符),不加任何前缀。",
  bracket: "预设「bracket」:正文是动词开头的陈述短句,类型名由程序拼成 [类型]。",
  custom: "预设「custom」:正文完全按下面的用户指令与示例来写。",
};

function typeTable(style: TitleStyle): string {
  return style.types
    .map((type) => `- ${type.emoji}  ${type.zh} / ${type.en}`)
    .join("\n");
}

function evidenceBlock(session: TitleEvidence): string {
  const lines = [
    `ref: ${session.ref}`,
    `agent: ${session.tool}`,
    ...(session.project ? [`项目: ${session.project}`] : []),
    ...(session.title ? [`当前标题: ${session.title}`] : []),
    `消息数: ${session.message_count ?? 0}`,
  ];
  for (const message of (session.user_messages ?? []).slice(0, 3)) {
    lines.push(`用户消息: ${message}`);
  }
  if (session.last_assistant_message) {
    lines.push(`最后的助手回复: ${session.last_assistant_message}`);
  }
  if (session.files?.length) {
    lines.push(`涉及文件: ${session.files.slice(0, 20).join(", ")}`);
  }
  return lines.join("\n");
}

export interface PromptRetryNote {
  ref: string;
  reason: string;
}

/**
 * 拼装顺序固定:硬约束 → 预设说明 → 用户指令 → 示例 → 会话证据 → 输出要求。
 * 用户指令排在预设之后,冲突时以用户为准。
 */
export function buildTitlePrompt(
  style: TitleStyle,
  sessions: readonly TitleEvidence[],
  retryNotes: readonly PromptRetryNote[] = [],
): string {
  const mode = prefixMode(style);
  const sections: string[] = [];
  const constraints = [
    "你在给编程会话重新命名。每个会话给一个标题正文。",
    "硬约束:",
    "1. 正文必须是单行纯文本:不带引号、不带换行、不带结尾标点。",
    `2. 正文长度不超过 ${style.max_chars}(汉字与英文字符都按 1 计)。`,
    `3. ${LANGUAGE_RULE[style.language]}`,
    "4. 正文要说清这个会话具体做了什么,不要写成「会话记录」这类空话。",
    "5. 正文里不要自己加类型前缀或 emoji,前缀由程序拼接。",
  ];
  if (mode === "none") {
    constraints.push('6. type 一律填空字符串 ""。');
  } else {
    constraints.push("6. type 从下面的类型表里选一个,填该类型的 emoji:");
  }
  sections.push(constraints.join("\n"));
  if (mode !== "none") sections.push(typeTable(style));
  sections.push(PRESET_RULE[style.preset]);
  if (style.instructions) {
    sections.push(
      `用户指令(与上面的预设冲突时以用户指令为准):\n${style.instructions}`,
    );
  }
  if (style.examples.length > 0) {
    sections.push(
      `示例标题:\n${style.examples.map((item) => `- ${item}`).join("\n")}`,
    );
  }
  sections.push(
    `会话证据(共 ${sessions.length} 个):\n\n${sessions
      .map((session) => evidenceBlock(session))
      .join("\n\n")}`,
  );
  if (retryNotes.length > 0) {
    sections.push(
      `上一轮这些会话的结果不可用,请重新生成并避开同样的问题:\n${retryNotes
        .map((note) => `- ${note.ref}: ${note.reason}`)
        .join("\n")}`,
    );
  }
  sections.push(
    [
      "输出要求:只输出一个 JSON 数组,不要 Markdown 代码块、不要任何解释。",
      '数组每项是 {"ref": "...", "type": "...", "title": "...", "skip": false, "reason": null}。',
      "ref 必须原样抄回。信息太少无法命名时该项填 skip: true 并在 reason 里写明原因。",
      `数组必须恰好包含这 ${sessions.length} 个 ref,顺序不限。`,
    ].join("\n"),
  );
  return sections.join("\n\n");
}
