import { test, vi } from "vitest";
import assert from "node:assert/strict";
import { render } from "@testing-library/react";

import { useSessionEditingSurface } from "./sessionEditing.jsx";

function Probe() {
  const { dirtyOps } = useSessionEditingSurface();
  return <span>{`待应用 ${dirtyOps.length}`}</span>;
}

test("缺 Provider 时在渲染期抛错", () => {
  const silenced = vi.spyOn(console, "error").mockImplementation(() => {});
  assert.throws(() => render(<Probe />), /SessionEditingProvider/);
  silenced.mockRestore();
});

