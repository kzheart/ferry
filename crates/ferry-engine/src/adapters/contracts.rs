//! 内置 Adapter 的静态能力契约。
//!
//! Python 的 `Protocol` 在 Rust 里落成 trait；5 个「鸭子类型可选方法」
//! （`scan_fingerprint` / `post_scan_maintenance` / `watch_stamp` /
//! `authoritative_members` / `load_preview`）建模为返回 `Option` 的默认方法，
//! `None` 表示该 adapter 没有提供这个能力，调用方按 Python 的 `getattr(..., None)`
//! 分支处理。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use serde_json::{Map, Value};

use crate::adapters::shared::editing::EditDocument;
use crate::contracts::agents::AGENT_CAPABILITIES;
use crate::errors::{DomainError, DomainResult};
use crate::events::Event;
use crate::model::{Session, ToolCall};
use crate::system::paths::{expanduser, is_within, realpath_strict};

/// 扫描行：adapter 输出的原生会话摘要（Python 的 `dict`）。
pub type ScanRow = Map<String, Value>;

/// 指纹：内容不透明，调用方只做相等性比较。
pub type Fingerprint = Value;

/// Adapter 可插拔组件的枚举，等价 Python `_COMPONENT_CAPABILITIES` 的键。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Component {
    Browser,
    MigrationSource,
    MigrationTarget,
    Editor,
    Verifier,
    Lifecycle,
    Models,
}

impl Component {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::MigrationSource => "migration_source",
            Self::MigrationTarget => "migration_target",
            Self::Editor => "editor",
            Self::Verifier => "verifier",
            Self::Lifecycle => "lifecycle",
            Self::Models => "models",
        }
    }
}

/// 组件 → 触发它存在的能力集合（任一命中即要求该组件存在）。
pub const COMPONENT_CAPABILITIES: &[(Component, &[&str])] = &[
    (Component::Browser, &["browse"]),
    (Component::MigrationSource, &["migration-source"]),
    (Component::MigrationTarget, &["migration-target"]),
    (Component::Editor, &["edit"]),
    (Component::Verifier, &["prompt"]),
    (Component::Lifecycle, &["resume", "delete"]),
    (Component::Models, &["models"]),
];

fn capabilities_for(component: Component) -> &'static [&'static str] {
    COMPONENT_CAPABILITIES
        .iter()
        .find(|(name, _)| *name == component)
        .map(|(_, capabilities)| *capabilities)
        .unwrap_or(&[])
}

/// Agent 行为的单一事实源，可序列化下发给前端与 Rust host。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentManifest {
    pub id: String,
    pub display_name: String,
    pub icon: String,
    pub source_path: String,
    pub capabilities: Vec<String>,
    pub edit_operations: Vec<String>,
    /// launch descriptor 可执行文件白名单
    pub executables: Vec<String>,
    pub fallback_bin_dirs: Vec<String>,
}

impl AgentManifest {
    /// 按生成契约里的静态定义装配一个 manifest。
    pub fn from_contract(contract: &crate::contracts::agents::AgentContract) -> Self {
        let owned = |values: &[&str]| values.iter().map(|value| (*value).to_string()).collect();
        Self {
            id: contract.id.to_string(),
            display_name: contract.display_name.to_string(),
            icon: contract.icon.to_string(),
            source_path: contract.source_path.to_string(),
            capabilities: owned(contract.capabilities),
            edit_operations: owned(contract.edit_operations),
            executables: owned(contract.executables),
            fallback_bin_dirs: owned(contract.fallback_bin_dirs),
        }
    }

    /// 与 Python `to_dict()` 逐字段一致的 DTO。
    pub fn to_value(&self) -> Value {
        let list = |values: &[String]| {
            Value::Array(
                values
                    .iter()
                    .map(|value| Value::from(value.as_str()))
                    .collect(),
            )
        };
        let mut payload = Map::new();
        payload.insert("id".into(), Value::from(self.id.as_str()));
        payload.insert(
            "display_name".into(),
            Value::from(self.display_name.as_str()),
        );
        payload.insert("icon".into(), Value::from(self.icon.as_str()));
        payload.insert("source_path".into(), Value::from(self.source_path.as_str()));
        payload.insert("capabilities".into(), list(&self.capabilities));
        payload.insert("edit_operations".into(), list(&self.edit_operations));
        payload.insert("executables".into(), list(&self.executables));
        payload.insert("fallback_bin_dirs".into(), list(&self.fallback_bin_dirs));
        Value::Object(payload)
    }
}

/// 原生会话的存储形态。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageKind {
    File,
    Directory,
    Id,
}

impl StorageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Id => "id",
        }
    }
}

/// Adapter 内部的原生引用；不会离开 Engine。
///
/// 不变量：`canonical_ref` 非空，且 `storage_kind == Id` 当且仅当 `root is None`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSessionReference {
    canonical_ref: String,
    root: Option<String>,
    storage_kind: StorageKind,
}

impl NativeSessionReference {
    pub fn new(
        canonical_ref: impl Into<String>,
        root: Option<String>,
        storage_kind: StorageKind,
    ) -> Result<Self, &'static str> {
        let canonical_ref = canonical_ref.into();
        if canonical_ref.is_empty() || (storage_kind == StorageKind::Id) != root.is_none() {
            return Err("非法原生会话引用");
        }
        Ok(Self {
            canonical_ref,
            root,
            storage_kind,
        })
    }

    pub fn canonical_ref(&self) -> &str {
        &self.canonical_ref
    }

    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    pub fn storage_kind(&self) -> StorageKind {
        self.storage_kind
    }
}

/// 扫描根只有三个固定目录，逐个会话重解析一次在数千行时是纯浪费。
static RESOLVED_ROOTS: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn resolved_root(source_path: &str) -> Option<String> {
    if let Some(cached) = RESOLVED_ROOTS
        .lock()
        .expect("扫描根缓存锁中毒")
        .get(source_path)
    {
        return cached.clone();
    }
    let resolved = realpath_strict(&expanduser(source_path))
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    RESOLVED_ROOTS
        .lock()
        .expect("扫描根缓存锁中毒")
        .insert(source_path.to_string(), resolved.clone());
    resolved
}

/// 清空扫描根缓存（等价 `_resolved_root.cache_clear()`，测试用）。
pub fn clear_resolved_root_cache() {
    RESOLVED_ROOTS.lock().expect("扫描根缓存锁中毒").clear();
}

/// 校验受 Agent root 约束的文件或目录引用。
///
/// 五道门：realpath(strict) → 类型匹配 → `is_within(root)` →
/// `required_name` 成员存在且在目录内 → `resolve_ref` 恒等回环。
pub fn filesystem_reference(
    row: &ScanRow,
    source_path: &str,
    resolve_ref: &dyn Fn(&Path) -> Option<String>,
    kind: StorageKind,
    required_name: Option<&str>,
) -> Option<NativeSessionReference> {
    debug_assert!(matches!(kind, StorageKind::File | StorageKind::Directory));
    let raw = row
        .get("path")?
        .as_str()
        .filter(|value| !value.is_empty())?;
    let root = resolved_root(source_path)?;
    let path = realpath_strict(&expanduser(raw)).ok()?;
    let path_text = path.to_string_lossy().into_owned();
    let type_ok = match kind {
        StorageKind::File => path.is_file(),
        StorageKind::Directory => path.is_dir(),
        StorageKind::Id => false,
    };
    if !type_ok || !is_within(&path_text, &root) {
        return None;
    }
    if let Some(required_name) = required_name {
        if required_name.is_empty()
            || Path::new(required_name).file_name()?.to_str()? != required_name
        {
            return None;
        }
        let required = realpath_strict(&path.join(required_name)).ok()?;
        if !required.is_file() || !is_within(&required.to_string_lossy(), &path_text) {
            return None;
        }
    }
    let resolved = realpath_strict(Path::new(&resolve_ref(&path)?)).ok()?;
    if resolved != path {
        return None;
    }
    NativeSessionReference::new(path_text, Some(root), kind).ok()
}

/// 校验由 Adapter 管理的原生 ID。
pub fn id_reference(row: &ScanRow) -> Option<NativeSessionReference> {
    let raw = row.get("id")?.as_str()?;
    if raw.is_empty() || raw.chars().count() > 512 || raw.contains('\0') {
        return None;
    }
    NativeSessionReference::new(raw, None, StorageKind::Id).ok()
}

/// 扫描缓存：由 `sessions::scan_cache` 实现，依赖方向从 sessions 指向 adapters，
/// 保证 `adapters` 不引用 `sessions`（分层规则见 `adapters/mod.rs`）。
pub trait ScanCache: Send + Sync {
    /// 外层 `Option` 表示是否命中缓存；内层 `None` 表示「已知不是会话」。
    fn get(&self, path: &Path, stat: &crate::jsonutil::FileStat) -> Option<Option<ScanRow>>;
    fn put(&self, path: &Path, stat: &crate::jsonutil::FileStat, meta: Option<ScanRow>);
    fn get_digest(&self, path: &Path, stat: &crate::jsonutil::FileStat) -> Option<String>;
    fn put_digest(&self, path: &Path, stat: &crate::jsonutil::FileStat, digest: &str);
    fn flush(&self);
}

/// 读侧最小能力：扫描、读取、引用解析。
pub trait SessionBrowser: Send + Sync {
    fn scan(&self, cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>>;
    fn read(&self, reference: &str) -> DomainResult<Session>;

    /// Agent 读取路径的变体。
    ///
    /// Python 侧的调用点是 `getattr(browser, "read_agent", browser.read)(ref)`
    /// （`sessions/agent_read.py:61`）——**未提供就回落到 `read`**。这里用带默认
    /// 实现的 trait 方法复刻这条鸭子类型语义，五个内置 adapter 仍各自覆写。
    fn read_agent(&self, reference: &str) -> DomainResult<Session> {
        self.read(reference)
    }

    /// UI 分页浏览路径。默认复用 Agent 读取；只在浏览不需要子树时由 adapter
    /// 提供更轻的根会话读取，正式读取与迁移仍走 [`Self::read`]。
    fn read_browser(&self, reference: &str) -> DomainResult<Session> {
        self.read_agent(reference)
    }
    fn resolve_ref(&self, reference: &str) -> DomainResult<String>;
    fn fingerprint(&self, reference: &str) -> DomainResult<Fingerprint>;
    fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint>;
    fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference>;
    fn validate_read_scope(&self, reference: &NativeSessionReference) -> DomainResult<()>;

    /// 可选：扫描路径的指纹变体，容忍旧快照（opencode 的后台重建走它）。
    fn scan_fingerprint(&self, _reference: &str) -> Option<DomainResult<Fingerprint>> {
        None
    }

    /// 可选：扫描收尾维护（如 opencode 指纹索引后台重建）。
    fn post_scan_maintenance(&self) -> Option<DomainResult<()>> {
        None
    }

    /// 可选：廉价变更令牌，活索引用它替代整树 stat。
    fn watch_stamp(&self) -> Option<DomainResult<Value>> {
        None
    }

    /// 可选：目录型会话（grok bundle）的权威成员清单。
    fn authoritative_members(&self, _reference: &str) -> Option<DomainResult<Vec<String>>> {
        None
    }
}

pub trait MigrationSource: Send + Sync {
    fn export_tree(&self, reference: &str) -> DomainResult<Session>;
}

pub trait MigrationTarget: Send + Sync {
    /// 写入前置检查：目标端此刻能不能被写。
    ///
    /// 在 plan/preview 阶段就调用，把「目标 App 正在运行」之类的门禁提前到用户
    /// 选目标那一步，而不是等他走完四步、点了确认才拦。默认放行。
    fn preflight(&self) -> DomainResult<()> {
        Ok(())
    }

    fn plan(&self, session: &Session) -> DomainResult<Map<String, Value>>;
    fn preview(&self, session: &Session, cwd: Option<&str>) -> DomainResult<Map<String, Value>>;
    fn write(&self, session: &Session, cwd: &str) -> DomainResult<Map<String, Value>>;
    fn classify_tool_call(&self, tool_call: &ToolCall) -> String;
}

pub trait SessionEditor: Send + Sync {
    fn name(&self) -> &str;
    fn operations(&self) -> &[&str];

    fn load(&self, reference: &str) -> DomainResult<EditDocument>;
    /// 返回的 change 记录会原样进 `preview.changes` / `result.changes` 下发宿主，
    /// 形状是 `events.event(...)` 的结构化事件，不是纯文本列表。
    fn apply_ops(&self, doc: &mut EditDocument, ops: &[Value]) -> DomainResult<Vec<Event>>;

    /// 默认拒绝：对齐 `EditBackend.replace_reply` 抛 `OperationUnsupportedError`。
    fn replace_reply(
        &self,
        _doc: &mut EditDocument,
        _turn: &Value,
        _reply: &Value,
    ) -> DomainResult<Vec<Event>> {
        Err(DomainError::operation_unsupported(
            self.name(),
            "replace-assistant-reply",
            Some("inplace"),
        ))
    }

    fn validate(&self, doc: &EditDocument) -> DomainResult<()>;
    fn stats(&self, doc: &EditDocument) -> DomainResult<Map<String, Value>>;
    fn commit(&self, doc: &mut EditDocument) -> DomainResult<Map<String, Value>>;

    /// 默认不做快照；返回 `None` 时编辑事务必须拒写（方案 §2.4 第 23 条）。
    fn snapshot(
        &self,
        _doc: &EditDocument,
        _reason_code: &str,
        _extra: Option<&Map<String, Value>>,
    ) -> DomainResult<Option<PathBuf>> {
        Ok(None)
    }

    fn restore_snapshot(&self, snapshot: &Path, doc: &EditDocument) -> DomainResult<()>;
    fn saved_revision(
        &self,
        result: &Map<String, Value>,
        doc: &EditDocument,
    ) -> DomainResult<String>;

    /// 可选：预览专用的只读加载路径（codex / opencode 提供）。
    fn load_preview(&self, _reference: &str) -> Option<DomainResult<EditDocument>> {
        None
    }
}

pub trait SessionVerifier: Send + Sync {
    /// `timeout` 默认 360 秒（分发层默认参数，方案 §2.1 第 5 条）。
    fn prompt_session(
        &self,
        session_id: &str,
        cwd: Option<&str>,
        prompt: &str,
        model: Option<&str>,
        timeout: u64,
    ) -> DomainResult<Map<String, Value>>;
}

/// `discover()` 的返回三元组。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelDiscovery {
    pub rows: Vec<Map<String, Value>>,
    pub source: String,
    pub default: Option<String>,
}

pub trait ModelCatalog: Send + Sync {
    fn discover(&self) -> DomainResult<ModelDiscovery>;
    fn fallback(&self) -> Vec<Map<String, Value>>;
}

/// 会话生命周期策略：resume / 清理 / 校验引用 / 永久删除。
pub trait SessionLifecycle: Send + Sync {
    fn resume_descriptor(&self, session_id: &str, cwd: &str) -> DomainResult<Map<String, Value>>;
    fn cleanup(&self, session_id: &str, dest: &Path) -> DomainResult<()>;
    fn validation_ref(&self, session_id: &str, dest: &Path) -> DomainResult<String>;
    fn delete(&self, adapter: &AgentAdapter, reference: &str) -> DomainResult<Map<String, Value>>;
}

/// Adapter 组装体。构造时执行 Python `__post_init__` 的 5 条校验。
#[derive(Clone)]
pub struct AgentAdapter {
    pub manifest: AgentManifest,
    pub browser: Option<Arc<dyn SessionBrowser>>,
    pub migration_source: Option<Arc<dyn MigrationSource>>,
    pub migration_target: Option<Arc<dyn MigrationTarget>>,
    pub editor: Option<Arc<dyn SessionEditor>>,
    pub verifier: Option<Arc<dyn SessionVerifier>>,
    pub lifecycle: Option<Arc<dyn SessionLifecycle>>,
    pub models: Option<Arc<dyn ModelCatalog>>,
}

/// [`AgentAdapter`] 的组装器；字段过多，逐个 setter 比长参数列表清楚。
#[derive(Clone, Default)]
pub struct AgentAdapterBuilder {
    browser: Option<Arc<dyn SessionBrowser>>,
    migration_source: Option<Arc<dyn MigrationSource>>,
    migration_target: Option<Arc<dyn MigrationTarget>>,
    editor: Option<Arc<dyn SessionEditor>>,
    verifier: Option<Arc<dyn SessionVerifier>>,
    lifecycle: Option<Arc<dyn SessionLifecycle>>,
    models: Option<Arc<dyn ModelCatalog>>,
}

impl AgentAdapterBuilder {
    pub fn browser(mut self, value: Arc<dyn SessionBrowser>) -> Self {
        self.browser = Some(value);
        self
    }

    pub fn migration_source(mut self, value: Arc<dyn MigrationSource>) -> Self {
        self.migration_source = Some(value);
        self
    }

    pub fn migration_target(mut self, value: Arc<dyn MigrationTarget>) -> Self {
        self.migration_target = Some(value);
        self
    }

    pub fn editor(mut self, value: Arc<dyn SessionEditor>) -> Self {
        self.editor = Some(value);
        self
    }

    pub fn verifier(mut self, value: Arc<dyn SessionVerifier>) -> Self {
        self.verifier = Some(value);
        self
    }

    pub fn lifecycle(mut self, value: Arc<dyn SessionLifecycle>) -> Self {
        self.lifecycle = Some(value);
        self
    }

    pub fn models(mut self, value: Arc<dyn ModelCatalog>) -> Self {
        self.models = Some(value);
        self
    }

    pub fn build(self, manifest: AgentManifest) -> Result<AgentAdapter, String> {
        AgentAdapter {
            manifest,
            browser: self.browser,
            migration_source: self.migration_source,
            migration_target: self.migration_target,
            editor: self.editor,
            verifier: self.verifier,
            lifecycle: self.lifecycle,
            models: self.models,
        }
        .validated()
    }
}

impl AgentAdapter {
    pub fn builder() -> AgentAdapterBuilder {
        AgentAdapterBuilder::default()
    }

    pub fn id(&self) -> &str {
        &self.manifest.id
    }

    pub fn supports(&self, capability: &str) -> bool {
        self.manifest
            .capabilities
            .iter()
            .any(|declared| declared == capability)
    }

    pub fn has_component(&self, component: Component) -> bool {
        match component {
            Component::Browser => self.browser.is_some(),
            Component::MigrationSource => self.migration_source.is_some(),
            Component::MigrationTarget => self.migration_target.is_some(),
            Component::Editor => self.editor.is_some(),
            Component::Verifier => self.verifier.is_some(),
            Component::Lifecycle => self.lifecycle.is_some(),
            Component::Models => self.models.is_some(),
        }
    }

    /// 能力未知 / 组件未知 / 能力与组件不匹配 / 未声明能力 / 组件缺席都失败。
    ///
    /// 五种失败一律折成 `AgentCapabilityError`：调用方只需要知道「这个 agent
    /// 干不了这件事」，具体是哪一种不构成不同的处理路径。
    pub fn require(&self, capability: &str, component: Component) -> DomainResult<()> {
        let known = AGENT_CAPABILITIES.contains(&capability);
        let mapped = capabilities_for(component).contains(&capability);
        if !known || !mapped || !self.supports(capability) || !self.has_component(component) {
            return Err(DomainError::agent_capability(self.id(), capability));
        }
        Ok(())
    }

    pub fn require_browser(&self) -> DomainResult<&dyn SessionBrowser> {
        self.require("browse", Component::Browser)?;
        Ok(self.browser.as_deref().expect("require 已校验组件存在"))
    }

    pub fn require_migration_source(&self) -> DomainResult<&dyn MigrationSource> {
        self.require("migration-source", Component::MigrationSource)?;
        Ok(self
            .migration_source
            .as_deref()
            .expect("require 已校验组件存在"))
    }

    pub fn require_migration_target(&self) -> DomainResult<&dyn MigrationTarget> {
        self.require("migration-target", Component::MigrationTarget)?;
        Ok(self
            .migration_target
            .as_deref()
            .expect("require 已校验组件存在"))
    }

    pub fn require_editor(&self) -> DomainResult<&dyn SessionEditor> {
        self.require("edit", Component::Editor)?;
        Ok(self.editor.as_deref().expect("require 已校验组件存在"))
    }

    /// verifier 组件只服务 `prompt` capability。
    pub fn require_verifier(&self, capability: &str) -> DomainResult<&dyn SessionVerifier> {
        self.require(capability, Component::Verifier)?;
        Ok(self.verifier.as_deref().expect("require 已校验组件存在"))
    }

    /// `capability` 只能是 `resume` 或 `delete`。
    pub fn require_lifecycle(&self, capability: &str) -> DomainResult<&dyn SessionLifecycle> {
        self.require(capability, Component::Lifecycle)?;
        Ok(self.lifecycle.as_deref().expect("require 已校验组件存在"))
    }

    pub fn require_models(&self) -> DomainResult<&dyn ModelCatalog> {
        self.require("models", Component::Models)?;
        Ok(self.models.as_deref().expect("require 已校验组件存在"))
    }

    /// Python `__post_init__` 的 5 条校验。
    fn validated(self) -> Result<Self, String> {
        let capabilities = &self.manifest.capabilities;
        let ordered: Vec<&str> = AGENT_CAPABILITIES
            .iter()
            .copied()
            .filter(|capability| capabilities.iter().any(|declared| declared == capability))
            .collect();
        let deduplicated = {
            let mut seen: Vec<&str> = capabilities.iter().map(String::as_str).collect();
            seen.sort_unstable();
            let total = seen.len();
            seen.dedup();
            seen.len() == total
        };
        if !deduplicated
            || capabilities
                .iter()
                .any(|capability| !AGENT_CAPABILITIES.contains(&capability.as_str()))
            || capabilities
                .iter()
                .map(String::as_str)
                .ne(ordered.iter().copied())
        {
            return Err(format!("Adapter capability 契约无效: {}", self.manifest.id));
        }
        for (component, required_capabilities) in COMPONENT_CAPABILITIES {
            let expected = required_capabilities
                .iter()
                .any(|capability| self.supports(capability));
            if self.has_component(*component) != expected {
                return Err(format!(
                    "Adapter capability/component 不一致: {}.{}",
                    self.manifest.id,
                    component.as_str()
                ));
            }
        }
        let declares_edit = self.supports("edit");
        if !declares_edit && !self.manifest.edit_operations.is_empty() {
            return Err(format!(
                "Adapter 未声明 edit 但包含编辑操作: {}",
                self.manifest.id
            ));
        }
        if declares_edit && self.manifest.edit_operations.is_empty() {
            return Err(format!(
                "Adapter 声明 edit 但未包含编辑操作: {}",
                self.manifest.id
            ));
        }
        if let Some(editor) = self.editor.as_ref() {
            if editor.operations().iter().copied().ne(self
                .manifest
                .edit_operations
                .iter()
                .map(String::as_str))
            {
                return Err(format!(
                    "Adapter 编辑操作契约不一致: {} manifest={:?}, editor={:?}",
                    self.manifest.id,
                    self.manifest.edit_operations,
                    editor.operations()
                ));
            }
        }
        Ok(self)
    }
}

impl std::fmt::Debug for AgentAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentAdapter")
            .field("manifest", &self.manifest)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::agents::agent;
    use serde_json::json;

    struct StubBrowser;

    impl SessionBrowser for StubBrowser {
        fn scan(&self, _cache: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
            Ok(Vec::new())
        }
        fn read(&self, _reference: &str) -> DomainResult<Session> {
            Ok(Session::new("stub", "s", "/tmp"))
        }
        fn read_agent(&self, reference: &str) -> DomainResult<Session> {
            self.read(reference)
        }
        fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
            Ok(reference.to_string())
        }
        fn fingerprint(&self, _reference: &str) -> DomainResult<Fingerprint> {
            Ok(Value::Null)
        }
        fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
            self.fingerprint(reference)
        }
        fn canonicalize(&self, _row: &ScanRow) -> Option<NativeSessionReference> {
            None
        }
        fn validate_read_scope(&self, _reference: &NativeSessionReference) -> DomainResult<()> {
            Ok(())
        }
    }

    fn manifest(id: &str, capabilities: &[&str], edit_operations: &[&str]) -> AgentManifest {
        AgentManifest {
            id: id.to_string(),
            display_name: id.to_string(),
            icon: id.to_string(),
            source_path: "~/.stub".into(),
            capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
            edit_operations: edit_operations
                .iter()
                .map(|value| value.to_string())
                .collect(),
            executables: Vec::new(),
            fallback_bin_dirs: Vec::new(),
        }
    }

    #[test]
    fn native_reference_invariants_are_enforced() {
        assert!(NativeSessionReference::new("", None, StorageKind::Id).is_err());
        // id 型必须没有 root。
        assert!(NativeSessionReference::new("x", Some("/root".into()), StorageKind::Id).is_err());
        // 文件/目录型必须有 root。
        assert!(NativeSessionReference::new("/root/a", None, StorageKind::File).is_err());
        let reference =
            NativeSessionReference::new("/root/a", Some("/root".into()), StorageKind::File)
                .unwrap();
        assert_eq!(reference.canonical_ref(), "/root/a");
        assert_eq!(reference.root(), Some("/root"));
        assert_eq!(reference.storage_kind(), StorageKind::File);
    }

    #[test]
    fn id_reference_rejects_oversized_and_nul_bearing_ids() {
        let row = |value: Value| {
            let mut row = ScanRow::new();
            row.insert("id".into(), value);
            row
        };
        assert!(id_reference(&row(json!("abc"))).is_some());
        assert!(id_reference(&row(json!(""))).is_none());
        assert!(id_reference(&row(json!("a\0b"))).is_none());
        assert!(id_reference(&row(json!("x".repeat(513)))).is_none());
        assert!(id_reference(&row(json!(1))).is_none());
        assert!(id_reference(&ScanRow::new()).is_none());
    }

    #[test]
    fn filesystem_reference_requires_an_identity_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let session = root.join("a.jsonl");
        std::fs::write(&session, "{}\n").unwrap();
        clear_resolved_root_cache();

        let mut row = ScanRow::new();
        row.insert(
            "path".into(),
            Value::from(session.to_string_lossy().as_ref()),
        );
        let identity = |path: &Path| Some(path.to_string_lossy().into_owned());

        let reference = filesystem_reference(
            &row,
            &root.to_string_lossy(),
            &identity,
            StorageKind::File,
            None,
        )
        .expect("同一路径必须通过恒等校验");
        assert_eq!(reference.storage_kind(), StorageKind::File);

        // resolve_ref 指向别处 → 拒绝。
        let elsewhere = root.join("b.jsonl");
        std::fs::write(&elsewhere, "{}\n").unwrap();
        let drifting = |_: &Path| Some(elsewhere.to_string_lossy().into_owned());
        assert!(filesystem_reference(
            &row,
            &root.to_string_lossy(),
            &drifting,
            StorageKind::File,
            None,
        )
        .is_none());

        // 目录型要求 → 类型不符即拒绝。
        assert!(filesystem_reference(
            &row,
            &root.to_string_lossy(),
            &identity,
            StorageKind::Directory,
            None,
        )
        .is_none());
    }

    #[test]
    fn filesystem_reference_validates_required_members() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let bundle = root.join("bundle");
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(bundle.join("meta.json"), "{}").unwrap();
        clear_resolved_root_cache();

        let mut row = ScanRow::new();
        row.insert(
            "path".into(),
            Value::from(bundle.to_string_lossy().as_ref()),
        );
        let identity = |path: &Path| Some(path.to_string_lossy().into_owned());
        let source = root.to_string_lossy().into_owned();

        assert!(filesystem_reference(
            &row,
            &source,
            &identity,
            StorageKind::Directory,
            Some("meta.json"),
        )
        .is_some());
        assert!(filesystem_reference(
            &row,
            &source,
            &identity,
            StorageKind::Directory,
            Some("missing.json"),
        )
        .is_none());
        // required_name 必须是纯 basename。
        assert!(filesystem_reference(
            &row,
            &source,
            &identity,
            StorageKind::Directory,
            Some("../meta.json"),
        )
        .is_none());
    }

    #[test]
    fn adapter_rejects_capability_component_mismatches() {
        // 声明 browse 却没有 browser。
        let error = AgentAdapter::builder()
            .build(manifest("stub", &["browse"], &[]))
            .unwrap_err();
        assert!(error.contains("capability/component 不一致"));

        // 顺序必须与 AGENT_CAPABILITIES 一致。
        let error = AgentAdapter::builder()
            .browser(Arc::new(StubBrowser))
            .build(manifest("stub", &["resume", "browse"], &[]))
            .unwrap_err();
        assert!(error.contains("capability 契约无效"));

        // 未声明 edit 却带编辑操作。
        let error = AgentAdapter::builder()
            .browser(Arc::new(StubBrowser))
            .build(manifest("stub", &["browse"], &["rewrite"]))
            .unwrap_err();
        assert!(error.contains("未声明 edit"));
    }

    #[test]
    fn require_maps_missing_capabilities_to_agent_capability_errors() {
        let adapter = AgentAdapter::builder()
            .browser(Arc::new(StubBrowser))
            .build(manifest("stub", &["browse"], &[]))
            .unwrap();
        assert!(adapter.require_browser().is_ok());
        let error = adapter
            .require_editor()
            .err()
            .expect("未声明 edit 必须失败");
        assert_eq!(error.code, "agent.request_invalid");
        assert_eq!(error.message(), "stub 不支持能力 edit");
        // 能力/组件映射不成立时同样拒绝。
        assert!(adapter.require("browse", Component::Editor).is_err());
        assert!(adapter
            .require("not-a-capability", Component::Browser)
            .is_err());
    }

    #[test]
    fn manifest_round_trips_the_generated_contract() {
        let contract = agent("claude").expect("claude 必须在生成契约里");
        let manifest = AgentManifest::from_contract(contract);
        let payload = manifest.to_value();
        assert_eq!(payload["id"], Value::from("claude"));
        assert_eq!(payload["display_name"], Value::from("Claude Code"));
        assert!(payload["capabilities"]
            .as_array()
            .unwrap()
            .contains(&Value::from("edit")));
    }
}
