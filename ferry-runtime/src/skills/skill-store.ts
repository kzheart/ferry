/**
 * 技能门面:组合技能库、候选发现与 skills.json 配置。
 * 「已安装」以目录存在为准,skills.json 里的 installed 只补来源标注。
 */
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import {
  SkillLibrary,
  normalizeSkillId,
  type SkillContent,
  type SkillEntry,
} from "./skill-library.js";
import {
  candidatePath,
  discover,
  normalizeScanSource,
  type SkillCandidate,
  type SkillSource,
} from "./skill-discovery.js";
import { readJsonFile, writeJsonAtomic } from "../storage/json-file.js";
import { WriteQueue } from "../storage/write-queue.js";

const SKILL_STORE_VERSION = 1 as const;

const MAX_GLOBAL = 64;
const MAX_SCAN_SOURCES = 16;
const MAX_CONFIG_BYTES = 1024 * 1024;

interface InstalledRecord {
  origin_label: string;
  origin_path: string;
  imported_at: string;
}

interface SkillConfig {
  schema_version: typeof SKILL_STORE_VERSION;
  global: string[];
  scan_sources: string[];
  installed: Record<string, InstalledRecord>;
}

interface SkillListing {
  skills: SkillEntry[];
  global: string[];
  scanSources: SkillSource[];
  configError?: string;
}

export interface SkillImportInput {
  candidateId?: string;
  path?: string;
  overwrite?: boolean;
}

export interface SkillStore {
  list(): Promise<SkillListing>;
  candidates(): Promise<{
    candidates: SkillCandidate[];
    sources: SkillSource[];
  }>;
  import(input: SkillImportInput): Promise<SkillEntry>;
  delete(id: string): Promise<void>;
  setGlobal(ids: string[]): Promise<string[]>;
  addSource(path: string): Promise<SkillSource[]>;
  removeSource(sourceId: string): Promise<SkillSource[]>;
  read(id: string): Promise<SkillContent>;
}

function emptyConfig(): SkillConfig {
  return {
    schema_version: SKILL_STORE_VERSION,
    global: [],
    scan_sources: [],
    installed: {},
  };
}

function parseConfig(value: unknown): SkillConfig {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("skill config is invalid");
  }
  const document = value as Record<string, unknown>;
  if (document.schema_version !== SKILL_STORE_VERSION) {
    throw new Error("unsupported skill config version");
  }
  const strings = (input: unknown, field: string, maximum: number) => {
    if (input === undefined) return [];
    if (!Array.isArray(input)) throw new Error(`${field} is invalid`);
    const values = input.map((item) => {
      if (typeof item !== "string" || item.length === 0 || item.length > 1024) {
        throw new Error(`${field} contains an invalid entry`);
      }
      return item;
    });
    return [...new Set(values)].slice(0, maximum);
  };
  const installed: Record<string, InstalledRecord> = {};
  if (document.installed !== undefined) {
    if (
      typeof document.installed !== "object" ||
      document.installed === null ||
      Array.isArray(document.installed)
    ) {
      throw new Error("skill installed table is invalid");
    }
    for (const [id, record] of Object.entries(
      document.installed as Record<string, unknown>,
    )) {
      if (typeof record !== "object" || record === null) continue;
      const item = record as Record<string, unknown>;
      installed[id] = {
        origin_label: String(item.origin_label ?? ""),
        origin_path: String(item.origin_path ?? ""),
        imported_at: String(item.imported_at ?? ""),
      };
    }
  }
  return {
    schema_version: SKILL_STORE_VERSION,
    global: strings(document.global, "skill global", MAX_GLOBAL),
    scan_sources: strings(
      document.scan_sources,
      "skill scan sources",
      MAX_SCAN_SOURCES,
    ),
    installed,
  };
}

abstract class BaseSkillStore implements SkillStore {
  protected config: SkillConfig = emptyConfig();
  protected configError: string | undefined;

  constructor(
    protected readonly library: SkillLibrary,
    /** 关掉后只扫用户自选目录;测试必须关,否则结果取决于开发者主目录。 */
    protected readonly includeBuiltinSources = true,
  ) {}

  protected scan() {
    return discover(this.config.scan_sources, this.includeBuiltinSources);
  }

  async list(): Promise<SkillListing> {
    await this.settled();
    const skills = await this.library.list(this.config.installed);
    const installed = new Set(skills.map((skill) => skill.id));
    return {
      skills,
      global: this.config.global.filter((id) => installed.has(id)),
      scanSources: (await this.scan()).sources,
      ...(this.configError ? { configError: this.configError } : {}),
    };
  }

  async candidates() {
    await this.settled();
    const { sources, candidates } = await this.scan();
    return { candidates, sources };
  }

  async import(input: SkillImportInput): Promise<SkillEntry> {
    await this.settled();
    const overwrite = input.overwrite === true;
    let sourceDir: string;
    let originLabel: string;
    if (input.candidateId) {
      const { candidates, sources } = await this.candidates();
      const candidate = candidatePath(candidates, input.candidateId);
      sourceDir = candidate.path;
      originLabel =
        sources.find((item) => item.id === candidate.source)?.label ??
        candidate.source;
    } else if (input.path) {
      sourceDir = normalizeScanSource(input.path);
      originLabel = "";
    } else {
      throw new Error("skill import needs candidate_id or path");
    }
    const desired = normalizeSkillId(basename(sourceDir));
    const id = await this.library.install(sourceDir, desired, overwrite);
    this.config.installed[id] = {
      origin_label: originLabel,
      origin_path: sourceDir,
      imported_at: new Date().toISOString(),
    };
    await this.changed();
    const skills = await this.library.list(this.config.installed);
    const entry = skills.find((skill) => skill.id === id);
    if (!entry) throw new Error("skill import failed");
    return entry;
  }

  async delete(id: string): Promise<void> {
    await this.settled();
    await this.library.remove(id);
    // 删目录的同时把 id 从通用集合里摘掉,否则会留下一个引用不到的幽灵
    this.config.global = this.config.global.filter((item) => item !== id);
    delete this.config.installed[id];
    await this.changed();
  }

  async setGlobal(ids: string[]): Promise<string[]> {
    await this.settled();
    const unique = [...new Set(ids)];
    if (unique.length > MAX_GLOBAL) throw new Error("too many global skills");
    const installed = new Set(
      (await this.library.list()).map((skill) => skill.id),
    );
    for (const id of unique) {
      if (!installed.has(id)) throw new Error(`skill ${id} is not installed`);
    }
    this.config.global = unique;
    await this.changed();
    return [...unique];
  }

  async addSource(path: string): Promise<SkillSource[]> {
    await this.settled();
    const normalized = normalizeScanSource(path);
    if (!this.config.scan_sources.includes(normalized)) {
      if (this.config.scan_sources.length >= MAX_SCAN_SOURCES) {
        throw new Error("too many scan sources");
      }
      this.config.scan_sources.push(normalized);
      await this.changed();
    }
    return (await this.scan()).sources;
  }

  async removeSource(sourceId: string): Promise<SkillSource[]> {
    await this.settled();
    const { sources } = await this.scan();
    const target = sources.find((source) => source.id === sourceId);
    if (!target) throw new Error("scan source not found");
    if (target.builtin)
      throw new Error("builtin scan source cannot be removed");
    this.config.scan_sources = this.config.scan_sources.filter(
      (path) => normalizeScanSource(path) !== target.path,
    );
    await this.changed();
    return (await this.scan()).sources;
  }

  read(id: string): Promise<SkillContent> {
    return this.library.read(id);
  }

  protected async settled(): Promise<void> {}

  protected abstract changed(): Promise<void>;
}

/**
 * 未注入文件存储时的进程内技能配置。
 * 库 root 默认落在临时目录——绝不碰真实的 ~/.ferry,免得测试污染用户数据。
 */
export class EphemeralSkillStore extends BaseSkillStore {
  constructor(
    root = join(tmpdir(), `ferry-skills-${randomUUID()}`),
    includeBuiltinSources = true,
  ) {
    super(new SkillLibrary(root), includeBuiltinSources);
  }

  protected async changed() {}
}

export class FileSkillStore extends BaseSkillStore {
  private readonly path: string;
  private readonly ready: Promise<void>;
  private readonly writes = new WriteQueue();

  constructor(dataDirectory: string, includeBuiltinSources = true) {
    super(
      new SkillLibrary(join(dataDirectory, "skills")),
      includeBuiltinSources,
    );
    this.path = join(dataDirectory, "skills.json");
    this.ready = this.load();
  }

  private async load() {
    let config: SkillConfig | undefined;
    try {
      config = await readJsonFile({
        path: this.path,
        maxBytes: MAX_CONFIG_BYTES,
        tooLargeMessage: "skill config is too large",
        parse: parseConfig,
      });
    } catch (error) {
      // 配置坏了不能让 runtime 起不来:回落空配置,把原因带给 UI
      this.config = emptyConfig();
      this.configError =
        error instanceof Error ? error.message : "skill config is invalid";
      return;
    }
    if (config) this.config = config;
    else await this.writeDisk();
  }

  private writeDisk() {
    return writeJsonAtomic(this.path, JSON.stringify(this.config, null, 2));
  }

  protected override async settled() {
    await this.ready;
    await this.writes.settled();
  }

  protected async changed() {
    await this.ready;
    await this.writes.run(() => this.writeDisk());
  }
}
