export function migrationPlanInput({
  sourceTool,
  ref,
  targetTool,
  maxTurn,
  probe = false,
  probeModel,
}) {
  const input = {
    kind: "migration",
    source_tool: sourceTool,
    ref,
    target_tool: targetTool,
    probe: !!probe,
  };
  if (maxTurn != null) input.max_turn = maxTurn;
  if (probe && probeModel) input.probe_model = probeModel;
  return input;
}

export const migrationPlanKey = input => JSON.stringify(input);

export function matchingMigrationPlan(planned, input) {
  return planned?.key === migrationPlanKey(input) ? planned.plan : null;
}
