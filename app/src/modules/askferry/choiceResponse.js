// 选择卡应答的三条规则:入参归一、乐观推进、失败回滚。放在钩子外面,useAskFerry
// 只留编排,这段本身也能单独测。
import { choiceRespond } from "../../platform/desktop/client.js";
import { patchChoice } from "./agentChatModel.js";

// 卡片是用户输入的出口,进来什么都得先削成契约形状再往宿主送
const normalize = answer => ({
  answered: answer?.answered !== false,
  selected: Array.isArray(answer?.selected)
    ? answer.selected.filter(value => typeof value === "string") : [],
  customText: typeof answer?.custom_text === "string" ? answer.custom_text : "",
});

export async function submitChoice(mutateLog, sessionId, requestId, answer) {
  const { answered, selected, customText } = normalize(answer);
  const patch = state => mutateLog(sessionId,
    log => patchChoice(log, requestId, { selected, customText, ...state }));
  patch({ status: answered ? "answered" : "unanswered", answered });
  try {
    return await choiceRespond(sessionId, requestId,
      { answered, selected, custom_text: customText });
  } catch (error) {
    // 宿主没收下这次应答,卡片就不能停在「已回答」:回到 pending 让用户能重试,
    // 已选项与自由输入原样留着,不让人重填一遍
    patch({ status: "pending", answered: false });
    throw error;
  }
}
