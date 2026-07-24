import {
  BUCKETS,
  bucketOf,
  fmtTime,
  repoOf,
} from "../../domain/sessions/sessionModel.js";
import { sessionIdentity } from "../../domain/sessions/sessionAttachment.js";

export function libraryToolCounts(sessions) {
  const counts = {};
  for (const session of sessions) {
    counts[session.tool] = (counts[session.tool] || 0) + 1;
  }
  return counts;
}

export function libraryDirectories(sessions) {
  return [...new Set(sessions.map(session => repoOf(session.dir)).filter(Boolean))].slice(0, 6);
}

export function libraryTags(metadata) {
  return [...new Set(Object.values(metadata).flatMap(item => item.tags || []))].slice(0, 12);
}

export function buildLibraryIndex({ sessions, metadata, migratedSessionKeys, t }) {
  return sessions.map(session => {
    const meta = metadata[sessionIdentity(session)] || {};
    const tags = meta.tags || [];
    const treeCount = session.tree_count || 1;
    return {
      tool: session.tool,
      bucket: bucketOf(session.updated),
      repo: repoOf(session.dir),
      tags,
      pinned: !!meta.pinned,
      sub: treeCount > 1,
      mig: migratedSessionKeys.has(sessionIdentity(session)),
      hay: `${session.title || ""}\n${meta.name || ""}\n${tags.join("\n")}\n${session.dir || ""}\n${session.id}`.toLowerCase(),
      row: {
        key: sessionIdentity(session),
        id: session.id,
        title: meta.name || session.title || t("app:library.untitled"),
        repo: repoOf(session.dir),
        dir: session.dir,
        active: fmtTime(session.updated, t),
        tool: session.tool,
        dot: "var(--ok)",
        pinned: !!meta.pinned,
        tags: meta.tags,
        hasSub: treeCount > 1,
        subLabel: t("app:library.subLabel", { n: treeCount - 1 }),
        hasMig: migratedSessionKeys.has(sessionIdentity(session)),
      },
    };
  });
}

const TIME_BUCKETS = {
  all: BUCKETS,
  today: ["today"],
  last7: ["today", "yesterday", "last7"],
  last30: ["today", "yesterday", "last7", "last30"],
};

export function buildLibraryGroups({ index, filter, query, t }) {
  const timeBuckets = TIME_BUCKETS[filter.time] || TIME_BUCKETS.all;
  const needle = query.trim().toLowerCase();
  const byKey = { pinned: [] };
  BUCKETS.forEach(key => { byKey[key] = []; });
  for (const entry of index) {
    const matches = filter.src.includes(entry.tool)
      && (!filter.tag || entry.tags.includes(filter.tag))
      && (!filter.dir || entry.repo === filter.dir)
      && (!filter.mig || entry.mig)
      && (!filter.sub || entry.sub)
      && (!needle || entry.hay.includes(needle));
    if (matches) (entry.pinned ? byKey.pinned : byKey[entry.bucket]).push(entry.row);
  }
  const groups = [];
  if (byKey.pinned.length) {
    groups.push({ key: "pinned", label: t("app:library.pinned"),
      count: byKey.pinned.length, rows: byKey.pinned });
  }
  for (const key of BUCKETS) {
    if (!timeBuckets.includes(key) || !byKey[key].length) continue;
    groups.push({ key, label: t(`common:bucket.${key}`),
      count: byKey[key].length, rows: byKey[key] });
  }
  return groups;
}

export function visibleLibraryIds(groups, collapsedGroups) {
  return groups.filter(group => !(collapsedGroups[group.key] ?? false))
    .flatMap(group => group.rows.map(row => row.key));
}

export function libraryFilterCount(filter, toolIds) {
  return (filter.src.length < toolIds.length ? 1 : 0)
    + (filter.time !== "all" ? 1 : 0)
    + (filter.dir ? 1 : 0)
    + (filter.mig ? 1 : 0)
    + (filter.sub ? 1 : 0)
    + (filter.tag ? 1 : 0);
}

export function libraryTokenDescriptors(filter, toolIds, toolNames, t) {
  const tokens = [];
  if (filter.src.length < toolIds.length) {
    filter.src.forEach(tool => tokens.push({ kind: "source", tool, label: toolNames[tool] }));
  }
  if (filter.time !== "all") {
    tokens.push({ kind: "time", label: t(`common:bucket.${filter.time}`) });
  }
  if (filter.dir) tokens.push({ kind: "dir", label: t("app:library.tokenDir", { dir: filter.dir }) });
  if (filter.mig) tokens.push({ kind: "mig", label: t("app:library.tokenOnlyMigrated") });
  if (filter.sub) tokens.push({ kind: "sub", label: t("app:library.tokenOnlySub") });
  if (filter.tag) tokens.push({ kind: "tag", label: t("app:library.tokenTag", { tag: filter.tag }) });
  return tokens;
}
