export function migrationPlanInput({
  sourceTool,
  ref,
  targetTool,
  maxTurn,
}) {
  const input = {
    kind: "migration",
    source_tool: sourceTool,
    ref,
    target_tool: targetTool,
  };
  if (maxTurn != null) input.max_turn = maxTurn;
  return input;
}

export const migrationPlanKey = input => JSON.stringify(input);

export function matchingMigrationPlan(planned, input) {
  return planned?.key === migrationPlanKey(input) ? planned.plan : null;
}
