/**
 * 候选发现:扫描外部 coding agent 的技能目录。
 * 只读——这里产出的东西不是 Ferry 的技能,必须经 SkillLibrary.install 复制进库才算数。
 */
import { realpath, stat } from "node:fs/promises";
import { loadSourcedSkills } from "@earendil-works/pi-agent-core";
import { homedir } from "node:os";
import {
  basename,
  dirname,
  isAbsolute,
  join,
  relative,
  resolve,
} from "node:path";
import {
  AGENT_IDS,
  AGENT_LABELS,
  AGENT_SKILL_PATHS,
  SHARED_SKILL_PATHS,
} from "../server/generated/agents.js";
import { SKILL_MANIFEST, SkillExecutionEnv } from "./skill-document.js";

const MAX_CANDIDATES_PER_SOURCE = 200;

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

function expandHome(input: string): string {
  const trimmed = input.trim();
  if (trimmed === "~") return homedir();
  if (trimmed.startsWith("~/")) return join(homedir(), trimmed.slice(2));
  return resolve(trimmed);
}

function builtinSources(): SkillSource[] {
  // 共享仓库排在最前:各 CLI 目录多半是软链农场,去重时把归属留给真身
  const sources: SkillSource[] = (SHARED_SKILL_PATHS as readonly string[]).map(
    (path, index) => ({
      id: SHARED_SKILL_PATHS.length > 1 ? `shared-${index + 1}` : "shared",
      label: path,
      path: expandHome(path),
      builtin: true,
      available: false,
    }),
  );
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

/** 同一份技能被多个 CLI 软链时只留一条:否则 go-plan 会在列表里出现三次。 */
async function dedupe(candidates: SkillCandidate[]): Promise<SkillCandidate[]> {
  const seen = new Set<string>();
  const unique: SkillCandidate[] = [];
  for (const candidate of candidates) {
    let key = candidate.path;
    try {
      key = await realpath(candidate.path);
    } catch {
      // 断链之类的读不到真身,退回按路径去重
    }
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(candidate);
  }
  return unique;
}

/** 来源状态不读取技能正文；会话解析与配置列表不应顺带扫描所有外部技能。 */
export async function listSources(
  scanSources: readonly string[] = [],
  includeBuiltin = true,
): Promise<SkillSource[]> {
  const sources = [
    ...(includeBuiltin ? builtinSources() : []),
    ...customSources(scanSources),
  ];
  for (const source of sources) {
    try {
      source.available = (await stat(source.path)).isDirectory();
    } catch {
      source.available = false;
    }
  }
  return sources;
}

/** 缺失目录只是 available:false；测试关闭内置来源，避免读取开发者的技能库。 */
export async function discover(
  scanSources: readonly string[] = [],
  includeBuiltin = true,
): Promise<{ sources: SkillSource[]; candidates: SkillCandidate[] }> {
  const sources = await listSources(scanSources, includeBuiltin);
  const loaded = await loadSourcedSkills(
    new SkillExecutionEnv(),
    sources
      .filter((source) => source.available)
      .map((source) => ({
        path: source.path,
        source,
      })),
  );
  const counts = new Map<string, number>();
  const candidates: SkillCandidate[] = [];
  for (const { skill, source } of loaded.skills) {
    // Ferry 导入的是完整技能目录，不把来源根目录下的普通 Markdown 当成独立技能。
    if (basename(skill.filePath) !== SKILL_MANIFEST) continue;
    const count = counts.get(source.id) ?? 0;
    if (count >= MAX_CANDIDATES_PER_SOURCE) continue;
    counts.set(source.id, count + 1);
    const directory = dirname(skill.filePath);
    candidates.push({
      candidateId: `${source.id}:${relative(source.path, directory) || "."}`,
      name: skill.name,
      description: skill.description,
      source: source.id,
      path: directory,
    });
  }
  return { sources, candidates: await dedupe(candidates) };
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
