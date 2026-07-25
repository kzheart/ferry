/**
 * 候选发现:扫描外部 coding agent 的技能目录。
 * 只读——这里产出的东西不是 Ferry 的技能,必须经 SkillLibrary.install 复制进库才算数。
 */
import { lstat, readFile, readdir } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";
import {
  AGENT_IDS,
  AGENT_LABELS,
  AGENT_SKILL_PATHS,
} from "../server/generated/agents.js";
import { SKILL_MANIFEST, parseSkillDocument } from "./skill-document.js";

const MAX_CANDIDATES_PER_SOURCE = 200;
const MAX_MANIFEST_BYTES = 256 * 1024;

export interface SkillSource {
  id: string;
  label: string;
  path: string;
  builtin: boolean;
  available: boolean;
}

export interface SkillCandidate {
  candidateId: string;
  name: string;
  description: string;
  source: string;
  path: string;
}

export function expandHome(input: string): string {
  const trimmed = input.trim();
  if (trimmed === "~") return homedir();
  if (trimmed.startsWith("~/")) return join(homedir(), trimmed.slice(2));
  return resolve(trimmed);
}

function builtinSources(): SkillSource[] {
  const sources: SkillSource[] = [];
  AGENT_IDS.forEach((id, index) => {
    const paths = AGENT_SKILL_PATHS[id] as readonly string[];
    paths.forEach((path, position) => {
      sources.push({
        id: paths.length > 1 ? `${id}-${position + 1}` : id,
        label: AGENT_LABELS[index] ?? id,
        path: expandHome(path),
        builtin: true,
        available: false,
      });
    });
  });
  return sources;
}

function customSources(scanSources: readonly string[]): SkillSource[] {
  return scanSources.map((path, index) => ({
    id: `custom-${index + 1}`,
    label: path,
    path: expandHome(path),
    builtin: false,
    available: false,
  }));
}

async function scan(source: SkillSource): Promise<SkillCandidate[]> {
  let items: string[];
  try {
    items = (await readdir(source.path, { withFileTypes: true }))
      .filter((item) => item.isDirectory() && !item.name.startsWith("."))
      .map((item) => item.name)
      .sort()
      .slice(0, MAX_CANDIDATES_PER_SOURCE);
  } catch {
    return [];
  }
  source.available = true;
  const candidates: SkillCandidate[] = [];
  for (const name of items) {
    const directory = join(source.path, name);
    const manifest = join(directory, SKILL_MANIFEST);
    try {
      const info = await lstat(manifest);
      if (!info.isFile() || info.size > MAX_MANIFEST_BYTES) continue;
      const document = parseSkillDocument(await readFile(manifest, "utf8"), name);
      candidates.push({
        candidateId: `${source.id}:${name}`,
        name: document.name,
        description: document.description,
        source: source.id,
        path: directory,
      });
    } catch {
      continue;
    }
  }
  return candidates;
}

/** 目录不存在(比如没装 Claude Code)只是 available:false,不是错误。 */
export async function discover(
  scanSources: readonly string[] = [],
): Promise<{ sources: SkillSource[]; candidates: SkillCandidate[] }> {
  const sources = [...builtinSources(), ...customSources(scanSources)];
  const candidates: SkillCandidate[] = [];
  for (const source of sources) {
    candidates.push(...(await scan(source)));
  }
  return { sources, candidates };
}

/** candidate_id 反解成候选目录;找不到就是找不到,绝不接受调用方直接给路径。 */
export function candidatePath(
  candidates: readonly SkillCandidate[],
  candidateId: string,
): SkillCandidate {
  const found = candidates.find((item) => item.candidateId === candidateId);
  if (!found) throw new Error("skill candidate not found");
  return found;
}

export function normalizeScanSource(input: string): string {
  const expanded = expandHome(input);
  if (!isAbsolute(expanded)) throw new Error("scan source must be absolute");
  return expanded;
}
