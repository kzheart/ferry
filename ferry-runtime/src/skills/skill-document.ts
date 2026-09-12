/** 技能格式由 pi 解析；Ferry 只保留本地导入的资源边界。 */
import {
  FileError,
  err,
  loadSkills,
  ok,
  type Result,
  type FileInfo,
} from "@earendil-works/pi-agent-core";
import { NodeExecutionEnv } from "@earendil-works/pi-agent-core/node";
import { join, resolve } from "node:path";

export const SKILL_MANIFEST = "SKILL.md";
export const MAX_MANIFEST_BYTES = 256 * 1024;

/** 每次扫描独立创建，防止外部技能软链循环导致递归无法结束。 */
export class SkillExecutionEnv extends NodeExecutionEnv {
  private readonly visitedDirectories = new Set<string>();

  constructor() {
    super({ cwd: process.cwd() });
  }

  override async listDir(
    path: string,
    signal?: AbortSignal,
  ): Promise<Result<FileInfo[], FileError>> {
    const canonical = await this.canonicalPath(path);
    if (!canonical.ok) return canonical;
    if (this.visitedDirectories.has(canonical.value)) return ok([]);
    this.visitedDirectories.add(canonical.value);
    return super.listDir(path, signal);
  }

  override async readTextFile(
    path: string,
    signal?: AbortSignal,
  ): Promise<Result<string, FileError>> {
    const info = await this.fileInfo(path);
    if (!info.ok) return info;
    if (info.value.size > MAX_MANIFEST_BYTES) {
      return err(new FileError("invalid", "skill document is too large", path));
    }
    return super.readTextFile(path, signal);
  }
}

export async function loadSkillDocument(directory: string) {
  const filePath = join(resolve(directory), SKILL_MANIFEST);
  const loaded = await loadSkills(new SkillExecutionEnv(), resolve(directory));
  const skill = loaded.skills.find((item) => item.filePath === filePath);
  if (!skill) {
    const diagnostic = loaded.diagnostics.find(
      (item) => item.path === filePath,
    );
    throw new Error(diagnostic?.message ?? "skill has no valid SKILL.md");
  }
  return skill;
}
