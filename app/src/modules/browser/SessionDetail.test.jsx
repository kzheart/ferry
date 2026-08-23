// 接续命令的成败反馈:拿不到命令时不能先报"已复制"——用户粘出来是空的,
// 却只会以为是自己哪里点错了。成败都落在按钮本身(本模块既有的反馈方式)。
import { afterEach, expect, test, vi } from "vitest";
import assert from "node:assert/strict";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

let descriptor = async () => ({ display_command: "claude --resume abc" });
let clipboard = async () => {};

vi.mock("../../shared/contracts/tools.js", async importOriginal => ({
  ...(await importOriginal()),
  resumeDescriptor: (...args) => descriptor(...args),
}));
vi.mock("../../platform/desktop/client.js", async importOriginal => ({
  ...(await importOriginal()),
  writeClipboardText: (...args) => clipboard(...args),
}));

const { SessionEditingProvider } = await import("../../shared/capabilities/sessionEditing.jsx");
const SessionDetail = (await import("./SessionDetail.jsx")).default;

afterEach(() => {
  descriptor = async () => ({ display_command: "claude --resume abc" });
  clipboard = async () => {};
  vi.useRealTimers();
});

const editing = {
  scope: null, setScope: () => {}, ops: [], dirtyOps: [],
  addOp: () => {}, removeOp: () => {}, updateOp: () => {},
  startReplyEdit: () => {}, replyEditError: null, onOpenDiff: () => {},
  onApply: () => {}, applying: false, onDiscardAll: () => {},
};

const meta = { id: "s1", tool: "claude", title: "一次会话", dir: "/tmp/proj", count: 2 };

function renderDetail(overrides = {}) {
  return render(
    <SessionEditingProvider value={editing}>
      <SessionDetail
        meta={meta}
        data={null}
        error={null}
        onOpenMigrate={() => {}}
        onRefresh={() => {}}
        refreshing={false}
        onResume={async () => {}}
        navigationTarget={null}
        onLoadMore={() => {}}
        loadingMore={false}
        {...overrides}
      />
    </SessionEditingProvider>,
  );
}

test("拿到接续命令才报已复制,且写进剪贴板的就是那条命令", async () => {
  const written = [];
  clipboard = async text => { written.push(text); };
  renderDetail();

  fireEvent.click(screen.getByTitle("browser:session.copyResume"));

  await waitFor(() => screen.getByTitle("browser:session.copiedResume"));
  assert.deepEqual(written, ["claude --resume abc"]);
});

test("取不到接续命令时报错,绝不报成已复制", async () => {
  descriptor = async () => { throw new Error("session file missing"); };
  const written = [];
  clipboard = async text => { written.push(text); };
  renderDetail();

  fireEvent.click(screen.getByTitle("browser:session.copyResume"));

  await waitFor(() => screen.getByTitle("browser:session.copyResumeFailed"));
  assert.equal(screen.queryByTitle("browser:session.copiedResume"), null);
  assert.deepEqual(written, [], "失败时不该往剪贴板写东西");
});

test("剪贴板本身写失败也算失败,不是已复制", async () => {
  clipboard = async () => { throw new Error("clipboard blocked"); };
  renderDetail();

  fireEvent.click(screen.getByTitle("browser:session.copyResume"));

  await waitFor(() => screen.getByTitle("browser:session.copyResumeFailed"));
  assert.equal(screen.queryByTitle("browser:session.copiedResume"), null);
});

test("打开终端失败时按钮给出失败态,而不是静静回到常态", async () => {
  renderDetail({ onResume: async () => { throw new Error("no terminal found"); } });

  fireEvent.click(screen.getByTitle("browser:session.resumeTerminal"));

  await waitFor(() => screen.getByTitle("browser:session.resumeFailed"));
});
