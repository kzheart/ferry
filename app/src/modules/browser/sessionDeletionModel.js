import { operationRef } from "./sessionModel.js";

const deleteInput = session => ({
  kind: "delete",
  tool: session.tool,
  refs: [operationRef(session)],
});

export async function prepareSessionDeletion(session, operationClient) {
  const plan = await operationClient.plan(deleteInput(session));
  return { session, plan };
}

export async function prepareSessionDeletions(sessions, operationClient) {
  const prepared = [];
  try {
    for (const session of sessions) {
      prepared.push(await prepareSessionDeletion(session, operationClient));
    }
    return prepared;
  } catch (error) {
    await cancelPreparedDeletions(prepared, operationClient);
    throw error;
  }
}

// 会话被钉住/归档/打标签时,计划期会把它挪进 excluded,删除名单为空
export const deleteIsBlocked = prepared =>
  (prepared?.plan?.preview?.excluded?.length || 0) > 0;

export const summarizePreparedDeletions = prepared => ({
  total: prepared.length,
});

export const applyPreparedDeletion = (prepared, operationClient) =>
  operationClient.apply(prepared.plan);

export async function cancelPreparedDeletions(prepared, operationClient) {
  await Promise.allSettled(prepared.map(({ plan }) =>
    operationClient.cancel(plan.plan_id)));
}
