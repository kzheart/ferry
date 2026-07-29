/** 角色定义与持久化存储。 */
import { FERRY_TOOL_NAMES, type FerryToolName } from "../tools/catalog.js";
import { SKILL_ID_PATTERN } from "../skills/skill-library.js";
import {
  type ModelSelection,
  type ThinkingLevel,
} from "../providers/provider-config.js";
import { parseThinkingLevel } from "../providers/provider-config-validation.js";
import { readJsonFile, writeJsonAtomic } from "../storage/json-file.js";
import { WriteQueue } from "../storage/write-queue.js";

export const ROLE_STORE_VERSION = 1 as const;
export const DEFAULT_ROLE_ID = "default";
export const SESSION_OPTIMIZER_ROLE_ID = "session-optimizer";

export type ApplyPolicy = "manual" | "auto";

export interface Role {
  id: string;
  name: string;
  description?: string;
  /** 展示用图标/配色标识,取值由前端图标库决定,运行时只保证格式合法。 */
  icon?: string;
  color?: string;
  persona: string;
  tools: FerryToolName[];
  /** 挂在这个角色上的技能 id;不校验是否存在——技能可能被删,失效 id 由 resolveFor 静默丢弃。 */
  skills: string[];
  apply_policy: ApplyPolicy;
  model?: ModelSelection;
  thinking?: ThinkingLevel;
  /** 显式标记该角色可用作会话优化器;只有标记的角色才会出现在优化器选择里。 */
  optimizer?: boolean;
  builtin: boolean;
}

/** skills 可省略:老的角色载荷不带这个键,parseRole 会补成空数组。 */
export type RoleInput = Omit<Role, "builtin" | "skills"> & {
  skills?: string[];
};

interface RoleDocument {
  schema_version: typeof ROLE_STORE_VERSION;
  roles: Role[];
  /** 内置角色的用户改写。单独成列:旧版本读到这个键会忽略它,回落到出厂内置角色而不是解析失败。 */
  builtin_overrides: Role[];
}

const MAX_ROLE_BYTES = 2 * 1024 * 1024;
const ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;
const MAX_ROLE_SKILLS = 64;
// 图标与配色是纯展示标识:运行时不认识具体图标名,只挡住畸形值,前端遇到未知名回落默认样式
const TOKEN_PATTERN = /^[a-z0-9-]{1,32}$/;

export const DEFAULT_ROLE: Role = Object.freeze({
  id: DEFAULT_ROLE_ID,
  name: "Ferry",
  description: "Ferry 默认助手",
  icon: "sparkles",
  color: "blue",
  persona: "",
  tools: [...FERRY_TOOL_NAMES],
  skills: [],
  apply_policy: "auto",
  builtin: true,
});

export const SESSION_OPTIMIZER_ROLE: Role = Object.freeze({
  id: SESSION_OPTIMIZER_ROLE_ID,
  name: "会话优化器",
  description: "改写会话中的用户提问,使其更清晰、更完整",
  icon: "wand-sparkles",
  color: "violet",
  persona:
    "你是会话优化器,负责把用户在历史会话里的提问改写得更清晰、更完整、更易被模型执行。" +
    "改写必须忠实于原始意图,不得虚构原文没有的背景、需求或约束。" +
    "只改写用户消息,不修改 assistant 回复;若改写后的提问可能与现存 assistant 回复不一致,必须明确提醒用户。",
  tools: [
    "session_search",
    "session_read",
    "session_edit",
  ] satisfies FerryToolName[],
  skills: [],
  apply_policy: "manual",
  optimizer: true,
  builtin: true,
});

/** 出厂内置角色:可以被改写,但改写只是覆盖层,原始定义永远留在这里供恢复默认。 */
const BUILTIN_ROLES: readonly Role[] = Object.freeze([
  DEFAULT_ROLE,
  SESSION_OPTIMIZER_ROLE,
]);

const builtinRole = (id: string) =>
  BUILTIN_ROLES.find((role) => role.id === id);

function text(value: unknown, field: string, maximum: number) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${field} must be a non-empty string`);
  }
  const result = value.trim();
  if (result.length > maximum) throw new Error(`${field} is too long`);
  return result;
}

function optionalText(value: unknown, field: string, maximum: number) {
  if (value === undefined) return undefined;
  return text(value, field, maximum);
}

function parseToken(value: unknown, field: string): string | undefined {
  if (value === undefined) return undefined;
  const token = text(value, field, 32);
  if (!TOKEN_PATTERN.test(token)) throw new Error(`${field} is invalid`);
  return token;
}

function parseModel(value: unknown): ModelSelection | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("role model is invalid");
  }
  const model = value as Record<string, unknown>;
  const thinking = parseThinkingLevel(model.thinking);
  return {
    provider: text(model.provider, "role model provider", 128),
    model: text(model.model, "role model id", 512),
    ...(thinking ? { thinking } : {}),
  };
}

function parseSkills(value: unknown): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error("role skills are invalid");
  const skills = value.map((skill) => {
    if (typeof skill !== "string" || !SKILL_ID_PATTERN.test(skill)) {
      throw new Error("role skills contain an invalid skill id");
    }
    return skill;
  });
  if (new Set(skills).size !== skills.length) {
    throw new Error("role skills contain duplicates");
  }
  if (skills.length > MAX_ROLE_SKILLS) {
    throw new Error("role has too many skills");
  }
  return skills;
}

function parseRole(value: unknown, builtin = false): Role {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("role is invalid");
  }
  const input = value as Record<string, unknown>;
  const id = text(input.id, "role id", 128);
  if (!ID_PATTERN.test(id)) throw new Error("role id is invalid");
  if (!Array.isArray(input.tools)) throw new Error("role tools are invalid");
  const tools = input.tools.map((tool) => {
    if (
      typeof tool !== "string" ||
      !(FERRY_TOOL_NAMES as readonly string[]).includes(tool)
    ) {
      throw new Error("role tools contain an unknown tool");
    }
    return tool as FerryToolName;
  });
  if (new Set(tools).size !== tools.length) {
    throw new Error("role tools contain duplicates");
  }
  if (typeof input.persona !== "string" || input.persona.length > 20_000) {
    throw new Error("role persona is invalid");
  }
  if (input.apply_policy !== "manual" && input.apply_policy !== "auto") {
    throw new Error("role apply_policy is invalid");
  }
  if (input.optimizer !== undefined && typeof input.optimizer !== "boolean") {
    throw new Error("role optimizer flag is invalid");
  }
  const skills = parseSkills(input.skills);
  const model = parseModel(input.model);
  const thinking = parseThinkingLevel(input.thinking);
  const description = optionalText(
    input.description,
    "role description",
    1_000,
  );
  const icon = parseToken(input.icon, "role icon");
  const color = parseToken(input.color, "role color");
  return {
    id,
    name: text(input.name, "role name", 200),
    ...(description ? { description } : {}),
    ...(icon ? { icon } : {}),
    ...(color ? { color } : {}),
    persona: input.persona,
    tools,
    skills,
    apply_policy: input.apply_policy,
    ...(model ? { model } : {}),
    ...(thinking ? { thinking } : {}),
    ...(input.optimizer === true ? { optimizer: true } : {}),
    builtin,
  };
}

function parseDocument(value: unknown): RoleDocument {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("role config is invalid");
  }
  const document = value as Record<string, unknown>;
  if (document.schema_version !== ROLE_STORE_VERSION) {
    throw new Error("unsupported role config version");
  }
  if (!Array.isArray(document.roles)) throw new Error("role list is invalid");
  const roles = document.roles.map((role) => parseRole(role, false));
  if (
    roles.some((role) => builtinRole(role.id)) ||
    new Set(roles.map((role) => role.id)).size !== roles.length
  ) {
    throw new Error("role ids must be unique and custom");
  }
  const source = document.builtin_overrides;
  if (source !== undefined && !Array.isArray(source)) {
    throw new Error("builtin role overrides are invalid");
  }
  const overrides = (source ?? []).map((role) => parseRole(role, true));
  if (
    overrides.some((role) => !builtinRole(role.id)) ||
    new Set(overrides.map((role) => role.id)).size !== overrides.length
  ) {
    throw new Error("builtin role overrides must target distinct builtins");
  }
  return {
    schema_version: ROLE_STORE_VERSION,
    roles,
    builtin_overrides: overrides,
  };
}

export interface RoleStore {
  list(): Promise<Role[]>;
  get(id: string): Promise<Role | undefined>;
  create(input: RoleInput): Promise<Role>;
  update(id: string, input: RoleInput): Promise<Role>;
  delete(id: string): Promise<void>;
  reset(id: string): Promise<Role>;
  copy(sourceId: string, input: { id: string; name?: string }): Promise<Role>;
}

abstract class BaseRoleStore implements RoleStore {
  protected roles: Role[] = [];
  protected overrides: Role[] = [];

  async list() {
    const builtins = BUILTIN_ROLES.map(
      (role) => this.overrides.find((item) => item.id === role.id) ?? role,
    );
    return structuredClone([...builtins, ...this.roles]);
  }

  async get(id: string) {
    return (await this.list()).find((role) => role.id === id);
  }

  async create(input: RoleInput) {
    const role = parseRole(input, false);
    if ((await this.get(role.id)) !== undefined) {
      throw new Error("role already exists");
    }
    this.roles.push(role);
    await this.changed();
    return structuredClone(role);
  }

  async update(id: string, input: RoleInput) {
    if (input.id !== id) throw new Error("role id cannot be changed");
    // 内置角色可改,但改动写进覆盖层,出厂定义原样保留给 reset 用
    const builtin = builtinRole(id) !== undefined;
    const target = builtin ? this.overrides : this.roles;
    const role = parseRole(input, builtin);
    const index = target.findIndex((item) => item.id === id);
    if (index >= 0) target[index] = role;
    else if (builtin) target.push(role);
    else throw new Error("role not found");
    await this.changed();
    return structuredClone(role);
  }

  async delete(id: string) {
    if (builtinRole(id)) throw new Error("builtin role cannot be deleted");
    const index = this.roles.findIndex((role) => role.id === id);
    if (index < 0) throw new Error("role not found");
    this.roles.splice(index, 1);
    await this.changed();
  }

  async reset(id: string) {
    const original = builtinRole(id);
    if (!original) throw new Error("role is not builtin");
    const index = this.overrides.findIndex((role) => role.id === id);
    if (index >= 0) {
      this.overrides.splice(index, 1);
      await this.changed();
    }
    return structuredClone(original);
  }

  async copy(sourceId: string, input: { id: string; name?: string }) {
    const source = await this.get(sourceId);
    if (!source) throw new Error("role not found");
    return this.create({
      ...source,
      id: input.id,
      name: input.name ?? `${source.name} 副本`,
    });
  }

  protected abstract changed(): Promise<void>;
}

/** 未注入文件存储时的进程内角色草稿；不跨 Runtime 生命周期保存。 */
export class EphemeralRoleStore extends BaseRoleStore {
  protected async changed() {}
}

export class FileRoleStore extends BaseRoleStore {
  private readonly ready: Promise<void>;
  private readonly writes = new WriteQueue();

  constructor(readonly path: string) {
    super();
    this.ready = this.load();
  }

  private async load() {
    const document = await readJsonFile({
      path: this.path,
      maxBytes: MAX_ROLE_BYTES,
      tooLargeMessage: "role config is too large",
      parse: parseDocument,
    });
    if (!document) {
      await this.writeDisk();
      return;
    }
    this.roles = document.roles;
    this.overrides = document.builtin_overrides;
  }

  private writeDisk() {
    const payload = JSON.stringify(
      {
        schema_version: ROLE_STORE_VERSION,
        roles: this.roles,
        builtin_overrides: this.overrides,
      },
      null,
      2,
    );
    return writeJsonAtomic(this.path, payload);
  }

  private async settled() {
    await this.ready;
    await this.writes.settled();
  }

  override async list() {
    await this.settled();
    return super.list();
  }

  override async update(id: string, input: RoleInput) {
    await this.ready;
    return super.update(id, input);
  }

  override async delete(id: string) {
    await this.ready;
    return super.delete(id);
  }

  override async reset(id: string) {
    await this.ready;
    return super.reset(id);
  }

  protected async changed() {
    await this.ready;
    await this.writes.run(() => this.writeDisk());
  }
}
