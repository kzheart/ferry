import { AGENTS } from "./generated/agents.js";

export const supportsEditOperation = (tool, operation) =>
  Boolean(AGENTS[tool]?.editOperations?.includes(operation));
