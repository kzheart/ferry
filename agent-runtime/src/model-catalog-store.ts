import { randomUUID } from "node:crypto";
import {
  chmod,
  mkdir,
  readFile,
  rename,
  unlink,
  writeFile,
} from "node:fs/promises";
import { join } from "node:path";
import type { ModelsStore, ModelsStoreEntry } from "@earendil-works/pi-ai";

const MAX_CATALOG_BYTES = 32 * 1024 * 1024;

function providerName(providerId: string) {
  if (!/^[a-z0-9][a-z0-9._-]{0,127}$/i.test(providerId)) {
    throw new Error("provider id is invalid");
  }
  return `${providerId}.json`;
}

function catalog(value: unknown): ModelsStoreEntry {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("model catalog is invalid");
  }
  const entry = value as Record<string, unknown>;
  if (!Array.isArray(entry.models)) {
    throw new Error("model catalog is invalid");
  }
  for (const key of ["lastModified", "checkedAt"]) {
    if (
      entry[key] !== undefined &&
      (!Number.isSafeInteger(entry[key]) || (entry[key] as number) < 0)
    ) {
      throw new Error("model catalog timestamp is invalid");
    }
  }
  return structuredClone(value) as ModelsStoreEntry;
}

export class FileModelsStore implements ModelsStore {
  private readonly writes = new Map<string, Promise<void>>();

  constructor(private readonly directory: string) {}

  async read(providerId: string) {
    const name = providerName(providerId);
    await this.writes.get(providerId)?.catch(() => undefined);
    try {
      const source = await readFile(join(this.directory, name), "utf8");
      if (Buffer.byteLength(source) > MAX_CATALOG_BYTES) {
        throw new Error("model catalog is too large");
      }
      return catalog(JSON.parse(source) as unknown);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
  }

  async write(providerId: string, entry: ModelsStoreEntry) {
    const name = providerName(providerId);
    const safe = catalog(entry);
    const payload = JSON.stringify(safe);
    if (Buffer.byteLength(payload) > MAX_CATALOG_BYTES) {
      throw new Error("model catalog is too large");
    }
    await this.enqueue(providerId, async () => {
      await mkdir(this.directory, { recursive: true, mode: 0o700 });
      const target = join(this.directory, name);
      const temporary = `${target}.${process.pid}.${randomUUID()}.tmp`;
      try {
        await writeFile(temporary, payload, { encoding: "utf8", mode: 0o600 });
        await rename(temporary, target);
        await chmod(target, 0o600);
      } catch (error) {
        await unlink(temporary).catch(() => undefined);
        throw error;
      }
    });
  }

  async delete(providerId: string) {
    const name = providerName(providerId);
    await this.enqueue(providerId, async () => {
      await unlink(join(this.directory, name)).catch((error) => {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      });
    });
  }

  private async enqueue(providerId: string, action: () => Promise<void>) {
    const previous = this.writes.get(providerId) ?? Promise.resolve();
    const next = previous.catch(() => undefined).then(action);
    this.writes.set(providerId, next);
    try {
      await next;
    } finally {
      if (this.writes.get(providerId) === next) this.writes.delete(providerId);
    }
  }
}
