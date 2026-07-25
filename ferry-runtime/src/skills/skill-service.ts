import { ProtocolError } from "../server/messages.js";
import type { SkillEntry } from "./skill-library.js";
import type { SkillImportInput, SkillStore } from "./skill-store.js";

export class SkillService {
  constructor(private readonly store: SkillStore) {}

  async list() {
    const listing = await this.guard(() => this.store.list());
    return {
      skills: listing.skills,
      global: listing.global,
      scan_sources: listing.scanSources,
      ...(listing.configError ? { config_error: listing.configError } : {}),
    };
  }

  candidates() {
    return this.guard(() => this.store.candidates());
  }

  async import(input: SkillImportInput) {
    return { skill: await this.guard(() => this.store.import(input)) };
  }

  async delete(id: string) {
    await this.guard(() => this.store.delete(id));
    return { skill_id: id, deleted: true };
  }

  async setGlobal(ids: string[]) {
    return { global: await this.guard(() => this.store.setGlobal(ids)) };
  }

  async addSource(path: string) {
    return { sources: await this.guard(() => this.store.addSource(path)) };
  }

  async removeSource(sourceId: string) {
    return {
      sources: await this.guard(() => this.store.removeSource(sourceId)),
    };
  }

  async read(id: string) {
    try {
      return await this.store.read(id);
    } catch (error) {
      throw new ProtocolError(
        "skill_not_found",
        error instanceof Error ? error.message : "skill not found",
      );
    }
  }

  /** 角色技能 ∪ 全局技能;未安装或 broken 的 id 静默丢弃——技能随时可能被删。 */
  async resolveFor(roleSkillIds: readonly string[]): Promise<SkillEntry[]> {
    let listing;
    try {
      listing = await this.store.list();
    } catch {
      return [];
    }
    const wanted = new Set([...roleSkillIds, ...listing.global]);
    return listing.skills
      .filter((skill) => wanted.has(skill.id) && !skill.broken)
      .sort((left, right) => left.name.localeCompare(right.name));
  }

  private async guard<T>(operation: () => Promise<T>) {
    try {
      return await operation();
    } catch (error) {
      throw new ProtocolError(
        "invalid_skill",
        error instanceof Error ? error.message : "skill request is invalid",
      );
    }
  }
}
