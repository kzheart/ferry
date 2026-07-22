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

function providerName(providerId: string) {
  if (!/^[a-z0-9][a-z0-9._-]{0,127}$/i.test(providerId)) {
    throw new Error("provider id is invalid");
  }
  return `${providerId}.json`;
}

export class FileModelsStore implements ModelsStore {
  private writeQueue = Promise.resolve();

  constructor(private readonly directory: string) {}

  async read(providerId: string) {
    const name = providerName(providerId);
    await this.writeQueue;
    try {
      return JSON.parse(
        await readFile(join(this.directory, name), "utf8"),
      ) as ModelsStoreEntry;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw error;
    }
  }

  async write(providerId: string, entry: ModelsStoreEntry) {
    const target = join(this.directory, providerName(providerId));
    const payload = JSON.stringify(entry);
    await this.enqueue(async () => {
      await mkdir(this.directory, { recursive: true, mode: 0o700 });
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
    const target = join(this.directory, providerName(providerId));
    await this.enqueue(async () => {
      await unlink(target).catch((error) => {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      });
    });
  }

  private enqueue(action: () => Promise<void>) {
    const next = this.writeQueue.catch(() => undefined).then(action);
    this.writeQueue = next;
    return next;
  }
}
