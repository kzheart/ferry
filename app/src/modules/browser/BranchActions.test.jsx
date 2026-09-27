import { afterEach, expect, test, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../platform/desktop/client.js", () => ({ engine: invoke }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key) => key }),
}));
import BranchActions from "./BranchActions.jsx";
const meta = { tool: "codex", id: "source", ref: "fsr_source" };
afterEach(() => {
  cleanup();
  invoke.mockReset();
});

test("copies the validated checkpoint and never asks native fork for resume", async () => {
  invoke.mockResolvedValue({ through: "fbp_point" });
  const resume = vi.fn().mockResolvedValue({ copied: true });
  render(
    <BranchActions
      meta={meta}
      round={{ locator: "fml_user" }}
      onResumeElsewhere={resume}
    />,
  );
  fireEvent.click(
    screen.getByRole("button", { name: "browser:branch.resume" }),
  );
  await waitFor(() => expect(resume).toHaveBeenCalledWith(meta, "fbp_point"));
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke).toHaveBeenCalledWith("branch_point", {
    tool: "codex",
    ref: "fsr_source",
    turn_locator: "fml_user",
  });
});

test("a lost fork response reuses its request ID and opens only the successful result", async () => {
  const requests = [];
  invoke.mockImplementation(async (method, params) => {
    if (method === "branch_point") return { through: "fbp_point" };
    requests.push(params);
    if (requests.length === 1) throw new Error("connection lost");
    return { id: "child", tool: "codex" };
  });
  const open = vi.fn();
  render(
    <BranchActions
      meta={meta}
      round={{ locator: "fml_user" }}
      onForkCreated={open}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "browser:branch.fork" }));
  await screen.findByRole("alert");
  expect(open).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "browser:branch.fork" }));
  await waitFor(() =>
    expect(open).toHaveBeenCalledWith({ id: "child", tool: "codex" }),
  );
  expect(requests).toHaveLength(2);
  expect(requests[0].request_id).toBe(requests[1].request_id);
});

test("Cursor exposes bounded resume without pretending to support native fork", () => {
  render(
    <BranchActions
      meta={{ ...meta, tool: "cursor" }}
      round={{ locator: "fml_user" }}
      onResumeElsewhere={() => {}}
    />,
  );
  expect(
    screen.queryByRole("button", { name: "browser:branch.fork" }),
  ).toBeNull();
  expect(
    screen.getByRole("button", { name: "browser:branch.resume" }),
  ).toBeTruthy();
});
