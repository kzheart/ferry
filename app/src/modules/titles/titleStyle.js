// 标题风格的读写与一次性试生成。风格的事实源在引擎的 title-style.json,
// 这里只保留一份默认值兜住"还没读回来"的那一帧。
import { engine, runtime } from "../../platform/desktop/client.js";
import { splitManualSessions } from "./titleResetModel.js";

export const TITLE_PRESETS = ["ferry", "english", "bracket", "custom"];
export const TITLE_LANGUAGES = ["follow", "zh", "en"];
export const TITLE_MAX_CHARS_RANGE = [6, 60];
export const TITLE_TYPE_LIMIT = 16;
export const TITLE_EXAMPLE_LIMIT = 3;

export const DEFAULT_TITLE_STYLE = Object.freeze({
  preset: "ferry",
  language: "follow",
  max_chars: 16,
  type_prefix: true,
  types: [
    { emoji: "✨", zh: "实现", en: "feat" },
    { emoji: "🐛", zh: "修复", en: "fix" },
    { emoji: "♻️", zh: "重构", en: "refactor" },
    { emoji: "🔍", zh: "调研", en: "research" },
    { emoji: "🧪", zh: "评估", en: "eval" },
    { emoji: "🩺", zh: "排查", en: "debug" },
    { emoji: "⚙️", zh: "配置", en: "config" },
    { emoji: "🚀", zh: "发布", en: "release" },
    { emoji: "📖", zh: "梳理", en: "docs" },
    { emoji: "💬", zh: "讨论", en: "discuss" },
  ],
  instructions: "",
  examples: [],
});

export const loadTitleStyle = () =>
  engine("title_style.get").then(result => result?.style || DEFAULT_TITLE_STYLE);

export const saveTitleStyle = style =>
  engine("title_style.set", { style }).then(result => result?.style || style);

export const titleEvidence = sessions =>
  engine("title_evidence", {
    sessions: sessions.map(session => ({ tool: session.tool, ref: session.ref })),
  });

/**
 * 取证 + 生成。style 传进来的是界面上当前(可能还没保存)的那份,
 * 不用证据里回带的已保存版本——「试一试」要试的正是改到一半的设置。
 */
export async function generateTitles({ sessions, includeManual, style }) {
  const evidence = await titleEvidence(sessions);
  const { candidates } = splitManualSessions(evidence, includeManual);
  if (!candidates.length) return { evidence, items: [], model: null };
  const generated = await runtime("title.generate", {
    sessions: candidates,
    style: style || evidence.style || DEFAULT_TITLE_STYLE,
  });
  return {
    evidence,
    items: generated?.items || [],
    model: generated?.model || null,
  };
}
