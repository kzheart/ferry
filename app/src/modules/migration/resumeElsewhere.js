// 「续聊到其他 agent」的纯逻辑:拼那条交给用户粘贴的指令,并给出复制后的反馈。
//
// 这里没有任何 RPC。续聊不经过引擎:用户在 Ferry 里复制一条指令,粘进任何装了
// ferry-resume skill 的 agent,由那边的 skill 自己用原生 id 换 `fsr_` ref、读历史、接着聊。
// 指令内容只取决于源会话,与粘到哪个 agent 无关——所以入口只有一个,不按目标分列。
import {
  integrationStatus,
  writeClipboardText,
} from "../../platform/desktop/client.js";

/**
 * 迁移倒在了「换个方式才有救」的地方吗?
 *
 * 两种:源存储被目标 agent 占着(Cursor 正在运行),以及目标压根不支持迁入。
 * 续聊零写入,这两种失败都绕得开;粘到哪个 agent 由用户决定,与迁移目标无关。
 */
export function canFallBackToResume(error) {
  if (!error) return false;
  const code = error.code || "";
  if (code === "session.store_unavailable") return true;
  return code === "agent.request_invalid"
    && String(error.params?.capability || "").startsWith("migration");
}

/**
 * 拼一条可以直接粘进目标 agent 的指令:`/ferry-resume <tool> <原生 id>`。
 *
 * 带的是**原生** session id 而不是 `fsr_` ref——ref 是引擎实例内临时签发的,
 * 粘贴时可能已经换了;skill 会自己用 `ferry search --session-id` 换回来。
 */
export function buildResumeCommand({ tool, sessionId } = {}) {
  const id = String(sessionId || "").trim();
  const source = String(tool || "").trim();
  if (!source || !id) return "";
  return `/ferry-resume ${source} ${id}`;
}

/**
 * 共享技能目录里装没装 ferry-resume。各 coding agent 共读这一个目录,所以
 * 只有一个目标要看。读不到宿主状态时返回 null:「不确定」和「没装」要区分
 * 对待,前者不该吓唬用户。
 */
async function skillInstalled() {
  let status;
  try {
    status = await integrationStatus();
  } catch {
    return null;
  }
  if (!Array.isArray(status?.skills) || status.skills.length === 0) return null;
  return status.skills.every(item => item.installed);
}

/**
 * 复制指令,反馈交给调用方渲染:按钮入口用它,把结果画在按钮自己身上。
 *
 * `copied` 表示剪贴板已写入;`noSkill` 只在确认没装 ferry-resume 时为 true
 * (读不到宿主状态时不吓唬用户);`error` 是写剪贴板失败的原因。
 */
export async function copyResumeInstruction({ tool, sessionId } = {}) {
  const command = buildResumeCommand({ tool, sessionId });
  if (!command) return { command: "", copied: false };
  try {
    await writeClipboardText(command);
  } catch (error) {
    return {
      command,
      copied: false,
      error: String(error?.message || error || ""),
    };
  }
  return { command, copied: true, noSkill: (await skillInstalled()) === false };
}

/**
 * 复制指令并给出反馈。副作用(剪贴板、toast、跳设置页)全部由调用方注入。
 *
 * 反馈分三档:装了就打勾说粘到哪都行;没装时不能打勾说「即可续聊」——粘过去
 * 根本不会被识别,得把「去安装」摆在前面;读不到状态就给一句中性的说明。
 */
export async function copyResumeCommand({
  tool, sessionId, t, setToast, openConfig,
}) {
  const result = await copyResumeInstruction({ tool, sessionId });
  if (!result.command) return "";
  if (!result.copied) {
    setToast?.({
      kind: "fail",
      title: t("app:toast.resumeCopyFail"),
      desc: result.error || "",
    });
    return "";
  }
  const command = result.command;
  if (result.noSkill) {
    setToast?.({
      kind: "warn",
      title: t("app:toast.resumeCopiedNoSkill"),
      desc: t("app:toast.resumeCopiedNoSkillDesc"),
      ...(openConfig
        ? {
          action: {
            label: t("app:toast.resumeInstallSkill"),
            onClick: () => openConfig("integration"),
          },
        }
        : {}),
    });
    return command;
  }
  setToast?.({
    kind: "ok",
    title: t("app:toast.resumeCopied"),
    desc: t("app:toast.resumeCopiedDesc"),
  });
  return command;
}
