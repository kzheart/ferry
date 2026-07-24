import { ProtocolError } from "../server/messages.js";
import type { RoleInput, RoleStore } from "./role-store.js";

export class RoleService {
  constructor(private readonly store: RoleStore) {}

  list() {
    return this.store.list();
  }

  async resolve(id: string) {
    const role = await this.store.get(id);
    if (!role) throw new ProtocolError("role_not_found", "role not found");
    return role;
  }

  create(input: RoleInput) {
    return this.mutate(() => this.store.create(input));
  }

  update(id: string, input: RoleInput) {
    return this.mutate(() => this.store.update(id, input));
  }

  async delete(id: string) {
    await this.mutate(() => this.store.delete(id));
    return { role_id: id, deleted: true };
  }

  copy(sourceId: string, id: string, name?: string) {
    return this.mutate(() =>
      this.store.copy(sourceId, { id, ...(name ? { name } : {}) }),
    );
  }

  private async mutate<T>(operation: () => Promise<T>) {
    try {
      return await operation();
    } catch (error) {
      throw new ProtocolError(
        "invalid_role",
        error instanceof Error ? error.message : "role is invalid",
      );
    }
  }
}
