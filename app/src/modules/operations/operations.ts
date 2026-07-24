import {
  operationApply,
  operationCancel,
  operationPlan,
  operationStatus,
} from "../../platform/desktop/client.js";
import { OperationController } from "./operationController.js";

export const operations = new OperationController({
  plan: operationPlan,
  apply: operationApply,
  status: operationStatus,
  cancel: operationCancel,
});
