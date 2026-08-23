import { mkdtemp, readFile, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_ROLE,
  FileRoleStore,
  ROLE_STORE_VERSION,
  type RoleInput,
} from "../src/roles/role-store.js";

function input(id: string): RoleInput {
  return {
    id,
    name: "只读分析师",
    description: "只允许检索",
    persona: "回答必须简洁。",
    tools: ["session_search", "session_read"],
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
      { id: "default", builtin: true, skills: [] },
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

  it("overrides a builtin role and restores it on reset", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-roles-"));
    const path = join(directory, "roles.json");
    const store = new FileRoleStore(path);

    await store.update("default", { ...input("default"), name: "我的 Ferry" });
    expect(await store.list()).toMatchObject([
      { id: "default", builtin: true, name: "我的 Ferry", thinking: "high" },
    ]);
    // 改写单独存一列,老版本读到只会忽略它,而不是把整份配置判为非法
    expect(JSON.parse(await readFile(path, "utf8"))).toMatchObject({
      roles: [],
      builtin_overrides: [{ id: "default", name: "我的 Ferry" }],
    });

    const restored = new FileRoleStore(path);
    expect(await restored.list()).toMatchObject([
      { name: "我的 Ferry" },
    ]);
    expect(await restored.reset("default")).toMatchObject({ name: "Ferry" });
    expect(await restored.list()).toMatchObject([
      { id: "default", builtin: true, name: "Ferry", persona: "" },
    ]);
    expect(JSON.parse(await readFile(path, "utf8"))).toMatchObject({
      builtin_overrides: [],
    });
  });

  it("protects builtin roles and rejects invalid or unknown tools", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-roles-"));
    const store = new FileRoleStore(join(directory, "roles.json"));

    await expect(store.delete("default")).rejects.toThrow("cannot be deleted");
    await expect(store.create(input("default"))).rejects.toThrow(
      "already exists",
    );
    await expect(store.reset("nope")).rejects.toThrow("not builtin");
    await expect(
      store.create({
        ...input("unsafe"),
        tools: ["shell"] as never,
      }),
    ).rejects.toThrow("unknown tool");

    await expect(
      store.create({
        ...input("agent-driver"),
        tools: ["agent_prompt"],
      }),
    ).resolves.toMatchObject({ tools: ["agent_prompt"] });
  });

  it("keeps icon and color as opaque display tokens", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-roles-"));
    const store = new FileRoleStore(join(directory, "roles.json"));

    expect(
      await store.create({ ...input("themed"), icon: "code", color: "violet" }),
    ).toMatchObject({ icon: "code", color: "violet" });
    // 运行时不维护图标白名单:未知图标名照样存下,由前端回落展示
    expect(
      await store.create({ ...input("future"), icon: "not-shipped-yet" }),
    ).toMatchObject({ icon: "not-shipped-yet" });
    await expect(
      store.create({ ...input("bad"), icon: "Code Icon!" }),
    ).rejects.toThrow("role icon is invalid");
  });

  it("角色技能是 id 列表,默认为空且拒绝非法 id", async () => {
    const directory = await mkdtemp(join(tmpdir(), "ferry-roles-"));
    const store = new FileRoleStore(join(directory, "roles.json"));

    // allow_bash 已经删除:bash 现在是能力清单里的一个工具,不再单独占一个开关
    expect(DEFAULT_ROLE).not.toHaveProperty("allow_bash");
    expect(DEFAULT_ROLE.skills).toEqual([]);
    expect(await store.create(input("plain"))).toMatchObject({ skills: [] });
    expect(
      await store.create({ ...input("skilled"), skills: ["code-review"] }),
    ).toMatchObject({ skills: ["code-review"] });

    await expect(
      store.create({ ...input("bad-skill"), skills: ["../escape"] }),
    ).rejects.toThrow("invalid skill id");
    await expect(
      store.create({ ...input("dup-skill"), skills: ["a", "a"] }),
    ).rejects.toThrow("duplicates");
  });
});
