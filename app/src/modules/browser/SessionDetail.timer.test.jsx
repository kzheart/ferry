// 这条用例需要 fake timer，单独隔离，避免与同文件前序用例留下的真实反馈 timer
// 混用后端。断言仍覆盖完整的失败态生命周期。
import { expect, test, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

vi.mock("../../shared/contracts/tools.js", async importOriginal => ({
  ...(await importOriginal()),
  resumeDescriptor: async () => { throw new Error("boom"); },
}));
vi.mock("../../platform/desktop/client.js", async importOriginal => ({
  ...(await importOriginal()),
  writeClipboardText: async () => {},
}));

const { SessionEditingProvider } = await import("../../shared/capabilities/sessionEditing.jsx");
const SessionDetail = (await import("./SessionDetail.jsx")).default;

const editing = {
  scope: null, setScope: () => {}, ops: [], dirtyOps: [],
  addOp: () => {}, removeOp: () => {}, updateOp: () => {},
  startReplyEdit: () => {}, replyEditError: null, onOpenDiff: () => {},
  onApply: () => {}, applying: false, onDiscardAll: () => {},
};

test("失败态会自行退去,按钮可以再试一次", async () => {
  render(
    <SessionEditingProvider value={editing}>
      <SessionDetail
        meta={{ id: "s1", tool: "claude", title: "一次会话", dir: "/tmp/proj", count: 2 }}
        data={null}
        error={null}
        onOpenMigrate={() => {}}
        onRefresh={() => {}}
        refreshing={false}
        onResume={async () => {}}
        navigationTarget={null}
        onLoadMore={() => {}}
        loadingMore={false}
        optimization={null}
      />
    </SessionEditingProvider>,
  );

  const nativeSetTimeout = globalThis.setTimeout;
  let resetFailure;
  const timer = vi.spyOn(globalThis, "setTimeout").mockImplementation(
    (callback, delay, ...args) => {
      if (delay === 4000) {
        resetFailure = callback;
        return 1;
      }
      return nativeSetTimeout(callback, delay, ...args);
    },
  );
  await act(async () => {
    fireEvent.click(screen.getByTitle("browser:session.copyResume"));
    await Promise.resolve();
  });
  expect(screen.getByTitle("browser:session.copyResumeFailed")).toBeTruthy();

  act(() => resetFailure());
  expect(screen.getByTitle("browser:session.copyResume")).toBeTruthy();
  timer.mockRestore();
});
