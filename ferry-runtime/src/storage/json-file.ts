/** JSON 配置文件的原子写与加载骨架,供各 store 复用。 */
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

export function isMissingFile(error: unknown) {
  return (error as NodeJS.ErrnoException).code === "ENOENT";
}

/** 临时文件 + rename 的原子写:中途失败就清掉临时文件,不留半截配置。 */
export async function writeJsonAtomic(path: string, payload: string) {
  const temporary = `${path}.${process.pid}.${randomUUID()}.tmp`;
  try {
    await writeFile(temporary, payload, { encoding: "utf8", mode: 0o600 });
    await rename(temporary, path);
    await chmod(path, 0o600);
  } catch (error) {
    await unlink(temporary).catch(() => undefined);
    throw error;
  }
}

interface ReadJsonOptions<T> {
  path: string;
  maxBytes: number;
  tooLargeMessage: string;
  parse: (value: unknown) => T;
}

/** 建目录后读取并解析;文件不存在返回 undefined,由调用方决定写默认值。 */
export async function readJsonFile<T>(
  options: ReadJsonOptions<T>,
): Promise<T | undefined> {
  await mkdir(dirname(options.path), { recursive: true, mode: 0o700 });
  try {
    const source = await readFile(options.path, "utf8");
    if (Buffer.byteLength(source) > options.maxBytes) {
      throw new Error(options.tooLargeMessage);
    }
    await chmod(options.path, 0o600);
    return options.parse(JSON.parse(source) as unknown);
  } catch (error) {
    if (isMissingFile(error)) return undefined;
    throw error;
  }
}
