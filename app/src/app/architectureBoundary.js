const domainSources = import.meta.glob("../domain/**/*.{js,jsx}", {
  eager: true,
  query: "?raw",
  import: "default",
});
const featureSources = import.meta.glob("../features/**/*.{js,jsx}", {
  eager: true,
  query: "?raw",
  import: "default",
});

const violations = [];
for (const [path, source] of Object.entries(domainSources)) {
  if (path.endsWith(".jsx")) violations.push(`${path}: domain 不允许 JSX 文件`);
  if (/\bfrom\s+["']react["']|\bimport\s*\(["']react["']\)/.test(source)) {
    violations.push(`${path}: domain 不允许依赖 React`);
  }
}
for (const [path, source] of Object.entries(featureSources)) {
  if (/\b(?:from\s+|import\s*\()["']@tauri-apps\//.test(source)) {
    violations.push(`${path}: feature 不允许直接依赖 Tauri`);
  }
}

if (violations.length) {
  throw new Error(`前端架构边界检查失败:\n${violations.join("\n")}`);
}
