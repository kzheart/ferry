// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
export const FERRY_EVENTS = Object.freeze({
  "auth.cancelled": {
    "source": "runtime",
    "forwardToUi": true
  },
  "auth.completed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "auth.event": {
    "source": "runtime",
    "forwardToUi": true
  },
  "auth.failed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "auth.prompt": {
    "source": "runtime",
    "forwardToUi": true
  },
  "content.delta": {
    "source": "runtime",
    "forwardToUi": true
  },
  "engine.request": {
    "source": "runtime",
    "forwardToUi": false
  },
  "operation.applied": {
    "source": "host",
    "forwardToUi": true
  },
  "operation.failed": {
    "source": "host",
    "forwardToUi": true
  },
  "operation.proposed": {
    "source": "host",
    "forwardToUi": true
  },
  "run.cancelled": {
    "source": "runtime",
    "forwardToUi": true
  },
  "run.completed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "run.failed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "run.interrupted": {
    "source": "runtime",
    "forwardToUi": true
  },
  "run.started": {
    "source": "runtime",
    "forwardToUi": true
  },
  "runtime.disconnected": {
    "source": "host",
    "forwardToUi": true
  },
  "session.created": {
    "source": "runtime",
    "forwardToUi": true
  },
  "session.model_changed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "task.cancelled": {
    "source": "runtime",
    "forwardToUi": true
  },
  "task.completed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "task.failed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "task.skipped": {
    "source": "runtime",
    "forwardToUi": true
  },
  "task.started": {
    "source": "runtime",
    "forwardToUi": true
  },
  "tool.completed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "tool.progress": {
    "source": "runtime",
    "forwardToUi": true
  },
  "tool.request": {
    "source": "runtime",
    "forwardToUi": true
  },
  "tool.started": {
    "source": "runtime",
    "forwardToUi": true
  },
  "user.message": {
    "source": "runtime",
    "forwardToUi": true
  },
  "workflow.completed": {
    "source": "runtime",
    "forwardToUi": true
  },
  "workflow.started": {
    "source": "runtime",
    "forwardToUi": true
  }
});
export const FERRY_EVENT_TYPES = Object.freeze(Object.keys(FERRY_EVENTS));
export const isFerryEventType = value =>
  typeof value === "string" &&
  Object.prototype.hasOwnProperty.call(FERRY_EVENTS, value);
