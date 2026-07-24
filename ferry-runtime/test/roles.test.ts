import { mkdtemp, readFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  FileRoleStore,
  ROLE_STORE_VERSION,
  type RoleInput,
} from "../src/roles/role-repository.js";

function input(id: string): RoleInput {
  return {
    id,
    name: "只读分析师",
    description: "只允许检索",
    persona: "回答必须简洁。",
    tools: ["session_search", "session_read"],
    allow_bash: false,
    apply_policy: "manual" as const,
    thinking: "high" as const,
  };
}

describe("FileRoleStore", () => {
  it("creates a versioned atomic private store and persists CRUD", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-roles-"));
    const path = join(directory, "roles.json");
    const store = new FileRoleStore(path);

    expect(await store.list()).toMatchObject([
      { id: "default", builtin: true, allow_bash: false },
    ]);
    await store.create(input("reader"));
    await store.update("reader", {
      ...input("reader"),
      name: "审阅者",
      persona: "只陈述证据。",
    });

    const copy = await store.copy("default", {
      id: "default-copy",
      name: "默认副本",
    });
    expect(copy).toMatchObject({ builtin: false, id: "default-copy" });
    await store.delete("reader");

    const restored = new FileRoleStore(path);
    await restored.update("default-copy", {
      ...input("default-copy"),
      name: "立即更新",
    });
    expect(await restored.list()).toMatchObject([
      { id: "default", builtin: true },
      { id: "default-copy", builtin: false, name: "立即更新" },
    ]);
    expect(JSON.parse(await readFile(path, "utf8"))).toMatchObject({
      schema_version: ROLE_STORE_VERSION,
      roles: [{ id: "default-copy", builtin: false, name: "立即更新" }],
    });
    expect((await stat(path)).mode & 0o777).toBe(0o600);
  });

  it("protects builtin roles and rejects invalid or unknown tools", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-roles-"));
    const store = new FileRoleStore(join(directory, "roles.json"));

    await expect(store.delete("default")).rejects.toThrow("cannot be deleted");
    await expect(
      store.update("default", { ...input("default") }),
    ).rejects.toThrow("immutable");
    await expect(
      store.create({
        ...input("unsafe"),
        tools: ["bash"] as never,
        allow_bash: true,
      }),
    ).rejects.toThrow("unknown tool");
  });
});
