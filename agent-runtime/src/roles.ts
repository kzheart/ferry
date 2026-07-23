import { randomUUID } from "node:crypto";
import {
  chmod,
  mkdir,
  readFile,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { dirname } from "node:path";
import { FERRY_TOOL_NAMES, type FerryToolName } from "./tool-port.js";
import {
  parseThinkingLevel,
  type ModelSelection,
  type ThinkingLevel,
} from "./provider-config.js";

export const ROLE_STORE_VERSION = 1 as const;
export const DEFAULT_ROLE_ID = "default";

export type ApplyPolicy = "manual" | "auto";

export interface Role {
  id: string;
  name: string;
  description?: string;
  persona: string;
  tools: FerryToolName[];
  allow_bash: boolean;
  apply_policy: ApplyPolicy;
  model?: ModelSelection;
  thinking?: ThinkingLevel;
  builtin: boolean;
}

export type RoleInput = Omit<Role, "builtin">;

interface RoleDocument {
  schema_version: typeof ROLE_STORE_VERSION;
  roles: Role[];
}

const MAX_ROLE_BYTES = 2 * 1024 * 1024;
const ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;

export const DEFAULT_ROLE: Role = Object.freeze({
  id: DEFAULT_ROLE_ID,
  name: "Ferry",
  description: "Ferry 默认助手",
  persona: "",
  tools: [...FERRY_TOOL_NAMES],
  allow_bash: false,
  apply_policy: "auto",
  builtin: true,
});

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

export function parseRole(value: unknown, builtin = false): Role {
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
  if (typeof input.allow_bash !== "boolean") {
    throw new Error("role allow_bash is invalid");
  }
  if (input.apply_policy !== "manual" && input.apply_policy !== "auto") {
    throw new Error("role apply_policy is invalid");
  }
  const model = parseModel(input.model);
  const thinking = parseThinkingLevel(input.thinking);
  const description = optionalText(
    input.description,
    "role description",
    1_000,
  );
  return {
    id,
    name: text(input.name, "role name", 200),
    ...(description ? { description } : {}),
    persona: input.persona,
    tools,
    allow_bash: input.allow_bash,
    apply_policy: input.apply_policy,
    ...(model ? { model } : {}),
    ...(thinking ? { thinking } : {}),
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
    roles.some((role) => role.id === DEFAULT_ROLE_ID) ||
    new Set(roles.map((role) => role.id)).size !== roles.length
  ) {
    throw new Error("role ids must be unique and custom");
  }
  return { schema_version: ROLE_STORE_VERSION, roles };
}

export interface RoleStore {
  list(): Promise<Role[]>;
  get(id: string): Promise<Role | undefined>;
  create(input: RoleInput): Promise<Role>;
  update(id: string, input: RoleInput): Promise<Role>;
  delete(id: string): Promise<void>;
  copy(sourceId: string, input: { id: string; name?: string }): Promise<Role>;
}

abstract class BaseRoleStore implements RoleStore {
  protected roles: Role[] = [];

  async list() {
    return structuredClone([DEFAULT_ROLE, ...this.roles]);
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
    if (id === DEFAULT_ROLE_ID) throw new Error("builtin role is immutable");
    if (input.id !== id) throw new Error("role id cannot be changed");
    const index = this.roles.findIndex((role) => role.id === id);
    if (index < 0) throw new Error("role not found");
    const role = parseRole(input, false);
    this.roles[index] = role;
    await this.changed();
    return structuredClone(role);
  }

  async delete(id: string) {
    if (id === DEFAULT_ROLE_ID)
      throw new Error("builtin role cannot be deleted");
    const index = this.roles.findIndex((role) => role.id === id);
    if (index < 0) throw new Error("role not found");
    this.roles.splice(index, 1);
    await this.changed();
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

export class MemoryRoleStore extends BaseRoleStore {
  protected async changed() {}
}

export class FileRoleStore extends BaseRoleStore {
  private readonly ready: Promise<void>;
  private writes = Promise.resolve();

  constructor(readonly path: string) {
    super();
    this.ready = this.load();
  }

  private async load() {
    await mkdir(dirname(this.path), { recursive: true, mode: 0o700 });
    try {
      const source = await readFile(this.path, "utf8");
      if (Buffer.byteLength(source) > MAX_ROLE_BYTES) {
        throw new Error("role config is too large");
      }
      this.roles = parseDocument(JSON.parse(source) as unknown).roles;
      await chmod(this.path, 0o600);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      await this.writeDisk();
    }
  }

  private async writeDisk() {
    const payload = JSON.stringify(
      { schema_version: ROLE_STORE_VERSION, roles: this.roles },
      null,
      2,
    );
    const temporary = `${this.path}.${process.pid}.${randomUUID()}.tmp`;
    try {
      await writeFile(temporary, payload, { encoding: "utf8", mode: 0o600 });
      await rename(temporary, this.path);
      await chmod(this.path, 0o600);
    } catch (error) {
      await unlink(temporary).catch(() => undefined);
      throw error;
    }
  }

  private async settled() {
    await this.ready;
    await this.writes;
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

  protected async changed() {
    await this.ready;
    const write = this.writes
      .catch(() => undefined)
      .then(() => this.writeDisk());
    this.writes = write;
    await write;
  }
}
