// 导出的是一个自描述的 bundle:kind 用来挡掉"随便挑了个 json 文件"的误操作,
// schema_version 跟随运行时的角色存储版本。builtin 是运行时算出来的,不进文件。

export const ROLE_BUNDLE_KIND = "ferry.roles";
export const ROLE_BUNDLE_VERSION = 1;

const FIELDS = ["id", "name", "description", "icon", "color", "persona", "tools",
  "skills", "apply_policy", "model", "thinking"];

const portable = role => Object.fromEntries(
  FIELDS.filter(field => role[field] !== undefined).map(field => [field, role[field]]));

export function buildRoleBundle(roles, now = new Date()) {
  return {
    kind: ROLE_BUNDLE_KIND,
    schema_version: ROLE_BUNDLE_VERSION,
    exported_at: now.toISOString(),
    roles: roles.map(portable),
  };
}

export const roleBundleFileName = roles => {
  const stem = roles.length === 1
    ? String(roles[0].id).replace(/[^A-Za-z0-9_-]/g, "") || "role"
    : "roles";
  return `ferry-${stem}.json`;
};

/** 解析导入文件;格式不对就抛出可直接展示的错误 key。 */
export function parseRoleBundle(text) {
  let value;
  try {
    value = JSON.parse(text);
  } catch {
    throw new Error("notJson");
  }
  // 也接受直接丢进来的角色配置文件本体(roles.json),它没有 kind 字段
  const isBundle = value && typeof value === "object" && !Array.isArray(value)
    && (value.kind === ROLE_BUNDLE_KIND || Array.isArray(value.roles));
  if (!isBundle) throw new Error("notBundle");
  if (value.kind && value.kind !== ROLE_BUNDLE_KIND) throw new Error("notBundle");
  if (value.schema_version !== undefined && value.schema_version !== ROLE_BUNDLE_VERSION) {
    throw new Error("badVersion");
  }
  const roles = (value.roles || []).filter(
    role => role && typeof role === "object" && !Array.isArray(role));
  if (!roles.length) throw new Error("empty");
  return roles.map(portable);
}

/**
 * 导入时的 ID 去重:与已有角色重名就追加 -2、-3……
 * 同一批文件内部也要去重,否则两个同 ID 的角色会在第二个上撞车。
 */
export function uniqueRoleId(candidate, taken) {
  const base = String(candidate || "role").replace(/[^A-Za-z0-9_-]/g, "").slice(0, 120) || "role";
  if (!taken.has(base)) return base;
  for (let n = 2; ; n += 1) {
    const next = `${base}-${n}`;
    if (!taken.has(next)) return next;
  }
}

export function planRoleImport(roles, existingIds) {
  const taken = new Set(existingIds);
  return roles.map(role => {
    const id = uniqueRoleId(role.id, taken);
    taken.add(id);
    return { role: { ...role, id }, renamedFrom: id === role.id ? null : role.id };
  });
}
