import { operationRef } from "./sessionModel.js";

const deleteInput = session => ({
  kind: "delete",
  tool: session.tool,
  ref: operationRef(session),
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

export const deleteIsUndoable = prepared =>
  prepared?.plan?.preview?.undoable === true;

export const deleteNeedsConfirmation = prepared =>
  !deleteIsUndoable(prepared)
  || Number(prepared?.session?.tree_count || 1) > 1;

export const summarizePreparedDeletions = prepared => {
  const undoable = prepared.filter(deleteIsUndoable).length;
  return {
    total: prepared.length,
    undoable,
    irreversible: prepared.length - undoable,
  };
};

export const applyPreparedDeletion = (prepared, operationClient) =>
  operationClient.apply(prepared.plan);

export async function cancelPreparedDeletions(prepared, operationClient) {
  await Promise.allSettled(prepared.map(({ plan }) =>
    operationClient.cancel(plan.plan_id)));
}
