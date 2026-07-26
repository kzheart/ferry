import { mkdir, readFile, unlink } from "node:fs/promises";
import { join } from "node:path";
import type { ModelsStore, ModelsStoreEntry } from "@earendil-works/pi-ai";
import { isMissingFile, writeJsonAtomic } from "../storage/json-file.js";
import { WriteQueue } from "../storage/write-queue.js";

function providerFileName(providerId: string) {
  if (!/^[a-z0-9][a-z0-9._-]{0,127}$/i.test(providerId)) {
    throw new Error("provider id is invalid");
  }
  return `${providerId}.json`;
}

export class FileModelsStore implements ModelsStore {
  private readonly writes = new WriteQueue();

  constructor(private readonly directory: string) {}

  async read(providerId: string) {
    const name = providerFileName(providerId);
    await this.writes.settled();
    try {
      return JSON.parse(
        await readFile(join(this.directory, name), "utf8"),
      ) as ModelsStoreEntry;
    } catch (error) {
      if (isMissingFile(error)) return undefined;
      throw error;
    }
  }

  async write(providerId: string, entry: ModelsStoreEntry) {
    const target = join(this.directory, providerFileName(providerId));
    const payload = JSON.stringify(entry);
    await this.writes.run(async () => {
      await mkdir(this.directory, { recursive: true, mode: 0o700 });
      await writeJsonAtomic(target, payload);
    });
  }

  async delete(providerId: string) {
    const target = join(this.directory, providerFileName(providerId));
    await this.writes.run(async () => {
      await unlink(target).catch((error) => {
        if (!isMissingFile(error)) throw error;
      });
    });
  }
}
