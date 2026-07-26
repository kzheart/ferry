/**
 * Ferry 技能库:唯一事实源。
 * 运行时只读 <dataDir>/skills,外部目录的技能必须先 install 复制进来才算数。
 */
import { randomUUID } from "node:crypto";
import {
  copyFile,
  lstat,
  mkdir,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
} from "node:fs/promises";
import { join, resolve, sep } from "node:path";
import { SKILL_MANIFEST, parseSkillDocument } from "./skill-document.js";

export const SKILL_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/;

/**
 * 导入边界:任一超限整体失败,不留半个技能在库里。
 * 数值按真实技能仓库量过——文档型技能上百个文件、媒体型技能几十 MB 都属常态,
 * 卡太紧会让一部分技能根本导不进来;这里只挡明显病态的目录树。
 */
const MAX_IMPORT_FILES = 2_000;
const MAX_IMPORT_BYTES = 64 * 1024 * 1024;
const MAX_IMPORT_DEPTH = 12;
const MAX_MANIFEST_BYTES = 256 * 1024;
const MAX_SKILLS = 200;

export interface SkillEntry {
  id: string;
  name: string;
  description: string;
  bytes: number;
  files: number;
  originLabel: string | null;
  broken: boolean;
}

export interface SkillContent {
  id: string;
  name: string;
  body: string;
  files: string[];
}

interface SkillOrigin {
  origin_label: string;
}

/** 把任意名字压成合法技能 id;压不出东西时回落 skill。 */
export function normalizeSkillId(value: string): string {
  const cleaned = value
    .trim()
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .replace(/^[^A-Za-z0-9]+/, "")
    .replace(/-+$/, "")
    .slice(0, 64);
  return SKILL_ID_PATTERN.test(cleaned) ? cleaned : "skill";
}

interface Measured {
  files: { relative: string; absolute: string }[];
  bytes: number;
}

/** 只统计普通文件;符号链接、设备文件与点开头的条目一律跳过。 */
async function measure(
  directory: string,
  depth: number,
  prefix: string,
  accumulator: Measured,
): Promise<void> {
  if (depth > MAX_IMPORT_DEPTH) {
    throw new Error("skill directory is nested too deeply");
  }
  for (const item of await readdir(directory)) {
    if (item.startsWith(".")) continue;
    const absolute = join(directory, item);
    const relative = prefix ? `${prefix}/${item}` : item;
    const info = await lstat(absolute);
    if (info.isSymbolicLink()) continue;
    if (info.isDirectory()) {
      await measure(absolute, depth + 1, relative, accumulator);
      continue;
    }
    if (!info.isFile()) continue;
    accumulator.files.push({ relative, absolute });
    accumulator.bytes += info.size;
    if (accumulator.files.length > MAX_IMPORT_FILES) {
      throw new Error("skill directory has too many files");
    }
    if (accumulator.bytes > MAX_IMPORT_BYTES) {
      throw new Error("skill directory is too large");
    }
  }
}

export class SkillLibrary {
  private readonly ready: Promise<void>;

  constructor(readonly root: string) {
    this.ready = mkdir(root, { recursive: true, mode: 0o700 }).then(
      () => undefined,
    );
  }

  /**
   * id 先过正则,再断言解析后的路径仍在 root 之内——路径穿越只有这一个入口要守。
   * 基准取 realpath 后的 root:macOS 上 /var 是 /private/var 的软链,不归一就永远对不上。
   */
  private async directoryOf(id: string) {
    if (!SKILL_ID_PATTERN.test(id)) throw new Error("skill id is invalid");
    const base = await realpath(this.root);
    const target = resolve(base, id);
    if (!target.startsWith(base + sep)) {
      throw new Error("skill id escapes the library root");
    }
    return target;
  }

  /** 只列目录名,不递归度量内容——只需要数量或 id 时别走 list()。 */
  private async listIds(): Promise<string[]> {
    await this.ready;
    return (await readdir(this.root, { withFileTypes: true }))
      .filter((item) => item.isDirectory() && !item.name.startsWith("."))
      .map((item) => item.name)
      .filter((name) => SKILL_ID_PATTERN.test(name))
      .sort();
  }

  async list(origins: Record<string, SkillOrigin> = {}): Promise<SkillEntry[]> {
    const names = await this.listIds();
    const entries: SkillEntry[] = [];
    for (const id of names) {
      entries.push(await this.describe(id, origins[id]?.origin_label ?? null));
    }
    return entries;
  }

  private async describe(
    id: string,
    originLabel: string | null,
  ): Promise<SkillEntry> {
    const directory = join(this.root, id);
    const accumulator: Measured = { files: [], bytes: 0 };
    let broken = false;
    try {
      await measure(directory, 1, "", accumulator);
    } catch {
      broken = true;
    }
    const manifest = accumulator.files.find(
      (file) => file.relative === SKILL_MANIFEST,
    );
    if (!manifest) {
      return {
        id,
        name: id,
        description: "",
        bytes: accumulator.bytes,
        files: accumulator.files.length,
        originLabel,
        broken: true,
      };
    }
    const source = await readFile(manifest.absolute, "utf8");
    const document = parseSkillDocument(source, id);
    return {
      id,
      name: document.name,
      description: document.description,
      bytes: accumulator.bytes,
      files: accumulator.files.length,
      originLabel,
      broken: broken || Buffer.byteLength(source) > MAX_MANIFEST_BYTES,
    };
  }

  async has(id: string): Promise<boolean> {
    if (!SKILL_ID_PATTERN.test(id)) return false;
    try {
      const info = await lstat(join(this.root, id));
      return info.isDirectory();
    } catch {
      return false;
    }
  }

  /**
   * 把 sourceDir 整目录复制成 <root>/<id>。
   * 先落到 .tmp-<uuid> 再 rename:中途超限就删临时目录,库里不会出现半个技能。
   */
  async install(
    sourceDir: string,
    desiredId: string,
    overwrite: boolean,
  ): Promise<string> {
    await this.ready;
    const source = await realpath(sourceDir);
    const info = await lstat(source);
    if (!info.isDirectory()) throw new Error("skill source is not a directory");
    const accumulator: Measured = { files: [], bytes: 0 };
    await measure(source, 1, "", accumulator);
    if (!accumulator.files.some((file) => file.relative === SKILL_MANIFEST)) {
      throw new Error("skill source has no SKILL.md");
    }
    const id = await this.claimId(normalizeSkillId(desiredId), overwrite);
    const staging = join(this.root, `.tmp-${randomUUID()}`);
    try {
      for (const file of accumulator.files) {
        const destination = join(staging, ...file.relative.split("/"));
        await mkdir(join(destination, ".."), { recursive: true, mode: 0o700 });
        await copyFile(file.absolute, destination);
      }
      const target = await this.directoryOf(id);
      if (overwrite) await rm(target, { recursive: true, force: true });
      await rename(staging, target);
    } catch (error) {
      await rm(staging, { recursive: true, force: true });
      throw error;
    }
    return id;
  }

  private async claimId(desired: string, overwrite: boolean) {
    const existing = await this.listIds();
    if (existing.length >= MAX_SKILLS) {
      throw new Error("skill library is full");
    }
    if (overwrite || !(await this.has(desired))) return desired;
    for (let suffix = 2; suffix <= 99; suffix += 1) {
      const candidate = normalizeSkillId(`${desired}-${suffix}`);
      if (!(await this.has(candidate))) return candidate;
    }
    throw new Error("skill id is taken");
  }

  async remove(id: string): Promise<void> {
    await this.ready;
    const directory = await this.directoryOf(id);
    if (!(await this.has(id))) throw new Error("skill not found");
    await rm(directory, { recursive: true, force: true });
  }

  async read(id: string): Promise<SkillContent> {
    await this.ready;
    const directory = await this.directoryOf(id);
    if (!(await this.has(id))) throw new Error("skill not found");
    const accumulator: Measured = { files: [], bytes: 0 };
    await measure(directory, 1, "", accumulator);
    const manifest = accumulator.files.find(
      (file) => file.relative === SKILL_MANIFEST,
    );
    if (!manifest) throw new Error("skill has no SKILL.md");
    const source = await readFile(manifest.absolute, "utf8");
    if (Buffer.byteLength(source) > MAX_MANIFEST_BYTES) {
      throw new Error("skill document is too large");
    }
    return {
      id,
      name: parseSkillDocument(source, id).name,
      body: source,
      files: accumulator.files.map((file) => file.relative).sort(),
    };
  }
}
