import { mkdir, mkdtemp, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { FileSkillStore } from "../src/skills/skill-store.js";
import { SkillService } from "../src/skills/skill-service.js";
import { discover } from "../src/skills/skill-discovery.js";

/** 造一个候选技能目录;所有测试都在临时目录里,绝不碰真实的 ~/.ferry。 */
async function candidateDirectory(
  root: string,
  name: string,
  description = "评审代码变更",
) {
  const directory = join(root, name);
  await mkdir(directory, { recursive: true });
  await writeFile(
    join(directory, "SKILL.md"),
    `---\nname: ${name} 技能\ndescription: ${description}\n---\n\n正文内容。\n`,
  );
  await writeFile(join(directory, "reference.md"), "附带资料\n");
  return directory;
}

async function fixture() {
  const base = await mkdtemp(join(tmpdir(), "ferry-skills-"));
  const data = join(base, "data");
  const external = join(base, "external");
  await mkdir(data, { recursive: true });
  await mkdir(external, { recursive: true });
  const store = new FileSkillStore(data);
  await store.addSource(external);
  return { base, data, external, store, service: new SkillService(store) };
}

describe("技能发现", () => {
  it("扫描只产出候选,既不写库也不动候选目录", async () => {
    const { data, external, store } = await fixture();
    const source = await candidateDirectory(external, "code-review");
    const before = (await stat(join(source, "SKILL.md"))).mtimeMs;

    const { candidates } = await store.candidates();
    expect(candidates.map((item) => item.name)).toContain("code-review 技能");

    const after = (await stat(join(source, "SKILL.md"))).mtimeMs;
    expect(after).toBe(before);
    expect(await readdir(join(source))).toEqual(["SKILL.md", "reference.md"]);
    // 扫描之后库仍然是空的——候选不是 Ferry 的技能
    expect((await store.list()).skills).toEqual([]);
    expect(await readdir(join(data, "skills"))).toEqual([]);
  });

  it("候选目录不存在时把来源标为 available:false 而不抛错", async () => {
    const missing = join(await mkdtemp(join(tmpdir(), "ferry-gone-")), "nope");
    const { sources } = await discover([missing]);
    const custom = sources.find((source) => !source.builtin);
    expect(custom?.available).toBe(false);
  });
});

describe("技能导入", () => {
  it("导入后技能进入库,正文可读", async () => {
    const { external, store, service } = await fixture();
    await candidateDirectory(external, "code-review");
    const { candidates } = await store.candidates();
    const candidate = candidates.find((item) => item.source.startsWith("custom"));

    const entry = await store.import({ candidateId: candidate!.candidateId });
    expect(entry.id).toBe("code-review");
    expect((await store.list()).skills.map((skill) => skill.id)).toEqual([
      "code-review",
    ]);

    const content = await service.read("code-review");
    expect(content.body).toContain("正文内容。");
    expect(content.files).toEqual(["SKILL.md", "reference.md"]);
  });

  it("导入后删掉上游目录,技能依旧可用", async () => {
    const { external, store, service } = await fixture();
    const source = await candidateDirectory(external, "code-review");
    const { candidates } = await store.candidates();
    await store.import({ candidateId: candidates[0]!.candidateId });

    await rm(source, { recursive: true, force: true });
    expect((await store.list()).skills.map((skill) => skill.id)).toEqual([
      "code-review",
    ]);
    expect((await service.read("code-review")).body).toContain("正文内容。");
  });

  it("同名再次导入生成 -2 后缀,overwrite 则原地覆盖", async () => {
    const { external, store } = await fixture();
    await candidateDirectory(external, "code-review");
    const { candidates } = await store.candidates();
    const id = candidates[0]!.candidateId;

    expect((await store.import({ candidateId: id })).id).toBe("code-review");
    expect((await store.import({ candidateId: id })).id).toBe("code-review-2");
    expect(
      (await store.import({ candidateId: id, overwrite: true })).id,
    ).toBe("code-review");
    expect((await store.list()).skills.map((skill) => skill.id)).toEqual([
      "code-review",
      "code-review-2",
    ]);
  });

  it("跳过符号链接,不把库外的东西复制进来", async () => {
    const { base, external, store } = await fixture();
    const source = await candidateDirectory(external, "linked");
    const secret = join(base, "secret.txt");
    await writeFile(secret, "不该被复制\n");
    await symlink(secret, join(source, "leak.txt"));

    const { candidates } = await store.candidates();
    await store.import({ candidateId: candidates[0]!.candidateId });
    const content = await store.read("linked");
    expect(content.files).not.toContain("leak.txt");
  });

  it("缺 SKILL.md 的目录不能导入", async () => {
    const { external, store } = await fixture();
    const directory = join(external, "empty");
    await mkdir(directory, { recursive: true });
    await expect(store.import({ path: directory })).rejects.toThrow();
  });
});

describe("技能配置", () => {
  it("read 未安装的 id 抛 skill_not_found", async () => {
    const { service } = await fixture();
    await expect(service.read("ghost")).rejects.toMatchObject({
      code: "skill_not_found",
    });
  });

  it("setGlobal 拒绝未安装的 id", async () => {
    const { service } = await fixture();
    await expect(service.setGlobal(["ghost"])).rejects.toMatchObject({
      code: "invalid_skill",
    });
  });

  it("delete 会把 id 从 global 里摘掉", async () => {
    const { external, store } = await fixture();
    await candidateDirectory(external, "code-review");
    const { candidates } = await store.candidates();
    await store.import({ candidateId: candidates[0]!.candidateId });

    expect(await store.setGlobal(["code-review"])).toEqual(["code-review"]);
    await store.delete("code-review");
    expect((await store.list()).global).toEqual([]);
    expect((await store.list()).skills).toEqual([]);
  });

  it("resolveFor 合并角色技能与全局技能,丢弃未安装的 id", async () => {
    const { external, store, service } = await fixture();
    await candidateDirectory(external, "code-review");
    await candidateDirectory(external, "pdf-tools");
    const { candidates } = await store.candidates();
    for (const candidate of candidates) {
      await store.import({ candidateId: candidate.candidateId });
    }
    await store.setGlobal(["pdf-tools"]);

    const resolved = await service.resolveFor(["code-review", "ghost"]);
    expect(resolved.map((skill) => skill.id).sort()).toEqual([
      "code-review",
      "pdf-tools",
    ]);
  });

  it("手工拖进库里的目录直接算已安装", async () => {
    const { data, store } = await fixture();
    await candidateDirectory(join(data, "skills"), "manual");
    const listing = await store.list();
    expect(listing.skills.map((skill) => skill.id)).toEqual(["manual"]);
    expect(listing.skills[0]?.originLabel).toBeNull();
  });

  it("内置扫描来源不可移除", async () => {
    const { store } = await fixture();
    await expect(store.removeSource("claude")).rejects.toThrow();
  });
});
