//! 声明式工具方言：每个 adapter 一份数据，读端归一与写端渲染由同一份声明推导。
//!
//! 设计约定：
//! - 一个 [`OpBinding`] 描述"某个规范操作在该 adapter 里叫什么、字段怎么对应"。
//!   九成映射是纯字段改名/单位换算（[`FieldMap`] + 命名转换器）；装不进表的怪例
//!   （多字段派生、列表拆装）用 `decode_hook` / `encode_hook` 显式声明。
//! - [`ToolDialect::parse`] 返回 `None` 表示"该调用无法无损归一"，调用方兜底成
//!   `tool.invoke` 私有调用，原始参数全量保留——归一失败不等于信息丢失。
//! - [`ToolDialect::render`] 返回 `None` 表示"目标端没有这个操作的原生形态"，
//!   由迁移层决定降级方式。
//! - 转换器是**有限枚举**（[`Converter`]）：未来的用户自定义映射只允许引用这些
//!   名字，不允许注入任意代码。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{LazyLock, RwLock};

use serde_json::{Map, Value};

use super::migration::Fidelity;

/// 命名转换器的有限枚举。`apply` 返回 `None` 等价 Python 的 `SKIP` 哨兵：
/// 该字段整体不写进结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Converter {
    /// `int(float(value))`；显式拒绝 bool。
    Int,
    /// `value if isinstance(value, str) else str(value)`。
    Str,
    /// 秒 → 毫秒（`int(value * 1000)`），非数值 SKIP。
    SToMs,
    /// 毫秒 → 秒（`value / 1000`，结果是 float），非数值 SKIP。
    MsToS,
    /// 真值 → `"dangerously-disable"`，否则 `"default"`。
    SandboxFlag,
    /// `value == "dangerously-disable"`。
    SandboxUnflag,
    /// bool 原样；`"true"`/`"false"`（不分大小写）转 bool；其余 SKIP。
    Bool,
}

impl Converter {
    /// 按 Python 的 `CONVERTERS` 键名查表。
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "int" => Some(Self::Int),
            "str" => Some(Self::Str),
            "s_to_ms" => Some(Self::SToMs),
            "ms_to_s" => Some(Self::MsToS),
            "sandbox_flag" => Some(Self::SandboxFlag),
            "sandbox_unflag" => Some(Self::SandboxUnflag),
            "bool" => Some(Self::Bool),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Str => "str",
            Self::SToMs => "s_to_ms",
            Self::MsToS => "ms_to_s",
            Self::SandboxFlag => "sandbox_flag",
            Self::SandboxUnflag => "sandbox_unflag",
            Self::Bool => "bool",
        }
    }

    /// `None` == Python 的 `SKIP`。
    pub fn apply(self, value: &Value) -> Option<Value> {
        match self {
            Self::Int => to_int(value),
            Self::Str => Some(Value::String(python_str(value))),
            Self::SToMs => numeric(value).map(|number| Value::from((number * 1000.0) as i64)),
            Self::MsToS => numeric(value).map(|number| json_float(number / 1000.0)),
            Self::SandboxFlag => Some(Value::from(if truthy(value) {
                "dangerously-disable"
            } else {
                "default"
            })),
            Self::SandboxUnflag => Some(Value::Bool(value.as_str() == Some("dangerously-disable"))),
            Self::Bool => match value {
                Value::Bool(flag) => Some(Value::Bool(*flag)),
                Value::String(text) => match text.to_lowercase().as_str() {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    _ => None,
                },
                _ => None,
            },
        }
    }
}

/// `_to_int`：bool 直接 SKIP（Python 里 `isinstance(True, int)` 为真，必须先拦）。
fn to_int(value: &Value) -> Option<Value> {
    match value {
        Value::Bool(_) => None,
        Value::Number(number) => {
            let float = number.as_f64()?;
            finite_trunc(float)
        }
        // `int(float("  12.7 "))`：Python 的 float() 容忍首尾空白。
        Value::String(text) => finite_trunc(text.trim().parse::<f64>().ok()?),
        _ => None,
    }
}

fn finite_trunc(float: f64) -> Option<Value> {
    // Python 的 `int(inf)` 抛 OverflowError（不在被捕获的异常里）；实际数据不会
    // 出现，这里按 SKIP 处理，避免 panic。
    if !float.is_finite() || float.abs() >= 9.223_372_036_854_776e18 {
        return None;
    }
    Some(Value::from(float.trunc() as i64))
}

/// `_numeric`：int/float 为真，bool 为假。
fn numeric(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        _ => None,
    }
}

/// Python 的 `/` 恒产出 float，`5000/1000` 序列化成 `5.0` 而不是 `5`。
fn json_float(float: f64) -> Value {
    serde_json::Number::from_f64(float).map_or(Value::Null, Value::Number)
}

/// Python `bool(value)` 的 JSON 等价。
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|float| float != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(entries) => !entries.is_empty(),
    }
}

/// `str(value)`：字符串原样，其余走 Python 的 `repr` 形态。
pub fn python_str(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => python_repr(other),
    }
}

/// `repr(value)`：容器里的字符串用单引号，bool/None 首字母大写。
pub fn python_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => {
            let escaped = text.replace('\\', "\\\\").replace('\'', "\\'");
            format!("'{escaped}'")
        }
        Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(python_repr).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .map(|(key, item)| {
                    format!(
                        "{}: {}",
                        python_repr(&Value::from(key.as_str())),
                        python_repr(item)
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

fn convert(converter: Option<Converter>, value: &Value) -> Option<Value> {
    match converter {
        None => Some(value.clone()),
        Some(converter) => converter.apply(value),
    }
}

/// 单个字段的双向对应。`native` 缺省时与 `canonical` 同名。
#[derive(Clone, Debug, Default)]
pub struct FieldMap {
    canonical: String,
    native: Option<String>,
    decode: Option<Converter>,
    encode: Option<Converter>,
    read_alt: Vec<String>,
    read_default: Option<Value>,
    write_default: Option<Value>,
    write: bool,
}

impl FieldMap {
    pub fn new(canonical: impl Into<String>) -> Self {
        Self {
            canonical: canonical.into(),
            write: true,
            ..Self::default()
        }
    }

    /// 原生字段名与规范名不同。
    pub fn native(mut self, native: impl Into<String>) -> Self {
        self.native = Some(native.into());
        self
    }

    pub fn decode(mut self, converter: Converter) -> Self {
        self.decode = Some(converter);
        self
    }

    pub fn encode(mut self, converter: Converter) -> Self {
        self.encode = Some(converter);
        self
    }

    /// 读端的备用原生名（旧代格式）。
    pub fn read_alt<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.read_alt = names.into_iter().map(Into::into).collect();
        self
    }

    pub fn read_default(mut self, value: impl Into<Value>) -> Self {
        self.read_default = Some(value.into());
        self
    }

    pub fn write_default(mut self, value: impl Into<Value>) -> Self {
        self.write_default = Some(value.into());
        self
    }

    /// `write=False`：只参与读端归一，写端不产出。
    pub fn no_write(mut self) -> Self {
        self.write = false;
        self
    }

    pub fn canonical_name(&self) -> &str {
        &self.canonical
    }

    pub fn native_name(&self) -> &str {
        self.native.as_deref().unwrap_or(&self.canonical)
    }

    /// `(native_name, *read_alt)`。
    pub fn read_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.native_name()).chain(self.read_alt.iter().map(String::as_str))
    }
}

/// 表外原生字段的处理策略。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Extras {
    /// 丢弃表外原生字段。
    #[default]
    Ignore,
    /// 整体退回 `tool.invoke`。
    Fallback,
}

/// `decode_hook`：原生入参 → 规范入参；`None` 表示无法无损归一。
pub type DecodeHook = fn(&Map<String, Value>) -> Option<Map<String, Value>>;
/// `encode_hook`：规范入参 → 原生入参；`None` 表示没有原生形态。
pub type EncodeHook = fn(&Map<String, Value>) -> Option<Map<String, Value>>;
/// `encode_post`：字段映射完成后的原生入参后处理（如把 workdir 内联进命令）。
pub type EncodePostHook = fn(&Map<String, Value>, Map<String, Value>) -> Map<String, Value>;
/// `render_flags`：渲染后追加的保真度声明。
pub type RenderFlagsHook = fn(&Map<String, Value>, &Map<String, Value>) -> RenderFlags;

/// `render_flags` 的返回值（Python 里是带 `_fidelity` / `_reason_codes` 的 dict）。
#[derive(Clone, Debug, Default)]
pub struct RenderFlags {
    pub fidelity: Option<Fidelity>,
    pub reason_codes: Vec<String>,
}

impl RenderFlags {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn new(fidelity: Fidelity, reason_codes: &[&str]) -> Self {
        Self {
            fidelity: Some(fidelity),
            reason_codes: reason_codes
                .iter()
                .map(|code| (*code).to_string())
                .collect(),
        }
    }
}

/// 一个规范操作在该 adapter 的原生形态。
#[derive(Clone, Debug)]
pub struct OpBinding {
    op: String,
    name: String,
    fields: Vec<FieldMap>,
    read_names: Vec<String>,
    extras: Extras,
    readonly: bool,
    decode_hook: Option<DecodeHook>,
    encode_hook: Option<EncodeHook>,
    render_flags: Option<RenderFlagsHook>,
    encode_post: Option<EncodePostHook>,
    encode_post_fields: Vec<String>,
}

impl OpBinding {
    pub fn new(op: impl Into<String>, name: impl Into<String>, fields: Vec<FieldMap>) -> Self {
        Self {
            op: op.into(),
            name: name.into(),
            fields,
            read_names: Vec::new(),
            extras: Extras::Ignore,
            readonly: false,
            decode_hook: None,
            encode_hook: None,
            render_flags: None,
            encode_post: None,
            encode_post_fields: Vec::new(),
        }
    }

    /// 读端还认这些原生工具名（旧代工具名等）。
    pub fn read_names<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.read_names = names.into_iter().map(Into::into).collect();
        self
    }

    pub fn extras(mut self, extras: Extras) -> Self {
        self.extras = extras;
        self
    }

    /// 只用于读端归一（如旧代工具名），写端不再产出。
    pub fn readonly(mut self) -> Self {
        self.readonly = true;
        self
    }

    pub fn decode_hook(mut self, hook: DecodeHook) -> Self {
        self.decode_hook = Some(hook);
        self
    }

    pub fn encode_hook(mut self, hook: EncodeHook) -> Self {
        self.encode_hook = Some(hook);
        self
    }

    pub fn render_flags(mut self, hook: RenderFlagsHook) -> Self {
        self.render_flags = Some(hook);
        self
    }

    /// `encode_post_fields` 声明后处理额外消化的规范字段，计入 supported。
    pub fn encode_post<I, S>(mut self, hook: EncodePostHook, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.encode_post = Some(hook);
        self.encode_post_fields = fields.into_iter().map(Into::into).collect();
        self
    }

    pub fn op(&self) -> &str {
        &self.op
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_readonly(&self) -> bool {
        self.readonly
    }

    pub fn render_flags_hook(&self) -> Option<RenderFlagsHook> {
        self.render_flags
    }

    /// `(name, *read_names)`。
    pub fn all_read_names(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.name.as_str()).chain(self.read_names.iter().map(String::as_str))
    }

    /// 原生入参 → 规范入参。
    pub fn parse(&self, raw: &Map<String, Value>) -> Option<Map<String, Value>> {
        if let Some(hook) = self.decode_hook {
            return hook(raw);
        }
        let mut canonical = Map::new();
        let mut consumed: HashSet<&str> = HashSet::new();
        for field in &self.fields {
            let key = field.read_names().find(|name| raw.contains_key(*name));
            match key {
                None => {
                    if let Some(default) = &field.read_default {
                        canonical.insert(field.canonical.clone(), default.clone());
                    }
                }
                Some(key) => {
                    consumed.insert(key);
                    if let Some(value) = convert(field.decode, &raw[key]) {
                        canonical.insert(field.canonical.clone(), value);
                    }
                }
            }
        }
        if self.extras == Extras::Fallback && raw.keys().any(|key| !consumed.contains(key.as_str()))
        {
            return None;
        }
        Some(canonical)
    }

    /// 规范入参 → 原生入参。
    pub fn render(&self, canonical: &Map<String, Value>) -> Option<Map<String, Value>> {
        if let Some(hook) = self.encode_hook {
            return hook(canonical);
        }
        let mut native = Map::new();
        for field in &self.fields {
            if !field.write {
                continue;
            }
            if let Some(value) = canonical.get(&field.canonical) {
                if let Some(converted) = convert(field.encode, value) {
                    native.insert(field.native_name().to_string(), converted);
                }
            } else if let Some(default) = &field.write_default {
                native.insert(field.native_name().to_string(), default.clone());
            }
        }
        if let Some(post) = self.encode_post {
            native = post(canonical, native);
        }
        Some(native)
    }

    /// 写端能消化的规范字段集合。
    pub fn supported_fields(&self) -> BTreeSet<String> {
        if self.encode_hook.is_some() || self.decode_hook.is_some() {
            return self
                .fields
                .iter()
                .map(|field| field.canonical.clone())
                .collect();
        }
        self.fields
            .iter()
            .filter(|field| field.write)
            .map(|field| field.canonical.clone())
            .chain(self.encode_post_fields.iter().cloned())
            .collect()
    }
}

/// 一个 adapter 的完整工具方言。
#[derive(Clone, Debug)]
pub struct ToolDialect {
    adapter: String,
    namespace: String,
    bindings: Vec<OpBinding>,
    strict_input: bool,
    drop_native: Vec<String>,
    by_read_name: HashMap<String, usize>,
    by_op: HashMap<String, usize>,
}

impl ToolDialect {
    /// `strict_input` 默认宽松、`drop_native` 默认为空；索引在这里一次性建好，
    /// 对齐 Python 的 `__post_init__`。
    pub fn new(
        adapter: impl Into<String>,
        namespace: impl Into<String>,
        bindings: Vec<OpBinding>,
    ) -> Self {
        let mut by_read_name = HashMap::new();
        let mut by_op: HashMap<String, usize> = HashMap::new();
        for (index, binding) in bindings.iter().enumerate() {
            for name in binding.all_read_names() {
                // 后声明的绑定覆盖同名的先声明者（对齐 dict 赋值）。
                by_read_name.insert(name.to_string(), index);
            }
            if !binding.readonly {
                // setdefault：同一个 op 的第一个可写绑定胜出。
                by_op.entry(binding.op.clone()).or_insert(index);
            }
        }
        Self {
            adapter: adapter.into(),
            namespace: namespace.into(),
            bindings,
            strict_input: false,
            drop_native: Vec::new(),
            by_read_name,
            by_op,
        }
    }

    /// 严格模式：入参不是 dict 时整体退回 `tool.invoke`（pi/grok）。
    /// 宽松模式：保留已识别的 op、原样透传入参（claude/opencode）。
    pub fn strict_input(mut self, strict: bool) -> Self {
        self.strict_input = strict;
        self
    }

    /// 解析前丢弃的传输层键（如 grok updates 流的 variant 判别符）：
    /// 它们是记录格式的痕迹而非调用参数，不参与 extras 守卫。
    pub fn drop_native<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.drop_native = names.into_iter().map(Into::into).collect();
        self
    }

    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn is_strict_input(&self) -> bool {
        self.strict_input
    }

    pub fn op_for(&self, name: &str) -> Option<&str> {
        self.by_read_name
            .get(name)
            .map(|index| self.bindings[*index].op.as_str())
    }

    /// 原生调用 → `(规范操作, 规范入参)`；`None` 表示应退回 `tool.invoke`。
    pub fn parse(&self, name: &str, raw: &Value) -> Option<(&str, Value)> {
        let binding = &self.bindings[*self.by_read_name.get(name)?];
        let Some(entries) = raw.as_object() else {
            return if self.strict_input {
                None
            } else {
                Some((binding.op.as_str(), raw.clone()))
            };
        };
        // null 值即"未设置"：有些格式（grok updates 流）会把完整参数 schema 连
        // null 一起写出，它们不携带信息，不该触发 extras 守卫。
        let filtered: Map<String, Value> = entries
            .iter()
            .filter(|(key, value)| {
                !value.is_null() && !self.drop_native.iter().any(|dropped| dropped == *key)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let canonical = binding.parse(&filtered)?;
        Some((binding.op.as_str(), Value::Object(canonical)))
    }

    /// 规范调用 → `(原生工具名, 原生入参)`；`None` 表示无原生形态。
    pub fn render(&self, op: &str, canonical: &Value) -> Option<(&str, Map<String, Value>)> {
        let binding = &self.bindings[*self.by_op.get(op)?];
        let entries = canonical.as_object()?;
        let native = binding.render(entries)?;
        Some((binding.name.as_str(), native))
    }

    pub fn binding_for(&self, op: &str) -> Option<&OpBinding> {
        self.by_op.get(op).map(|index| &self.bindings[*index])
    }

    pub fn write_ops(&self) -> BTreeSet<String> {
        self.by_op.keys().cloned().collect()
    }

    pub fn supported_fields(&self, op: &str) -> BTreeSet<String> {
        self.binding_for(op)
            .map(OpBinding::supported_fields)
            .unwrap_or_default()
    }
}

/// 目标端 shell 没有工作目录参数时，把 workdir 前缀成 `cd` 保住语义。
pub fn inline_workdir(
    canonical: &Map<String, Value>,
    native: Map<String, Value>,
) -> Map<String, Value> {
    inline_workdir_with_key(canonical, native, "command")
}

/// [`inline_workdir`] 的自定义命令字段版本。
pub fn inline_workdir_with_key(
    canonical: &Map<String, Value>,
    mut native: Map<String, Value>,
    command_key: &str,
) -> Map<String, Value> {
    let Some(workdir) = canonical.get("workdir") else {
        return native;
    };
    if !truthy(workdir) {
        return native;
    }
    let command = native.get(command_key).map(python_str).unwrap_or_default();
    let quoted = shell_quote(&python_str(workdir));
    native.insert(
        command_key.to_string(),
        Value::String(format!("cd {quoted} && {command}")),
    );
    native
}

/// workdir 被改写进命令时，把保真度如实标成 transformed。
pub fn workdir_inline_flags(
    canonical: &Map<String, Value>,
    _native: &Map<String, Value>,
) -> RenderFlags {
    match canonical.get("workdir") {
        Some(workdir) if truthy(workdir) => {
            RenderFlags::new(Fidelity::Transformed, &["workdir_inlined"])
        }
        _ => RenderFlags::none(),
    }
}

/// 等价 `shlex.quote`：安全字符集是 `[\w@%+=:,./-]`（ASCII）。
pub fn shell_quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    let safe = text
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character));
    if safe {
        return text.to_string();
    }
    format!("'{}'", text.replace('\'', "'\"'\"'"))
}

/// 进程级方言注册表。
///
/// Python 的 `get_dialect` 走 `importlib` 懒加载；Rust 没有 import 副作用，
/// 改为**静态注册**：每个 adapter 在自己的 `build()` 里调用一次
/// [`register_dialect`]，把 `static DIALECT: LazyLock<ToolDialect>` 的 `'static`
/// 引用登记进来。`registry::create_registry()` 会依次装配 5 个 adapter，因此
/// 服务起来之后 5 个槽位必然齐备；单测里各 adapter 自己注册自己的即可。
static DIALECTS: LazyLock<RwLock<BTreeMap<&'static str, &'static ToolDialect>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// 登记一个 adapter 的方言。重复登记同一个 adapter 以最后一次为准（幂等）。
pub fn register_dialect(adapter: &'static str, dialect: &'static ToolDialect) {
    DIALECTS
        .write()
        .expect("方言注册表锁中毒")
        .insert(adapter, dialect);
}

/// 按 adapter 名取方言；未注册返回 `None`（对齐 Python 的 `ModuleNotFoundError`）。
pub fn get_dialect(adapter: &str) -> Option<&'static ToolDialect> {
    DIALECTS
        .read()
        .expect("方言注册表锁中毒")
        .get(adapter)
        .copied()
}

/// 已注册的 adapter 名（装配自检用）。
pub fn registered_dialects() -> Vec<&'static str> {
    DIALECTS
        .read()
        .expect("方言注册表锁中毒")
        .keys()
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_ops::CanonicalOp;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    /// 结构与 claude 的 Bash 绑定一致，用于验证共享层语义。
    fn shell_dialect() -> ToolDialect {
        ToolDialect::new(
            "test",
            "test",
            vec![OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "Bash",
                vec![
                    FieldMap::new("command").read_default("").write_default(""),
                    FieldMap::new("timeout_ms").native("timeout"),
                    FieldMap::new("sandbox_policy")
                        .native("dangerouslyDisableSandbox")
                        .decode(Converter::SandboxFlag)
                        .encode(Converter::SandboxUnflag),
                    FieldMap::new("description"),
                ],
            )
            .encode_post(inline_workdir, ["workdir"])
            .render_flags(workdir_inline_flags)],
        )
    }

    #[test]
    fn converters_match_python_semantics() {
        assert_eq!(Converter::Int.apply(&json!("12.7")), Some(json!(12)));
        assert_eq!(Converter::Int.apply(&json!(-12.7)), Some(json!(-12)));
        assert_eq!(Converter::Int.apply(&json!(true)), None);
        assert_eq!(Converter::Int.apply(&json!("abc")), None);
        assert_eq!(Converter::Int.apply(&json!(null)), None);
        // Python 的 float() 容忍首尾空白。
        assert_eq!(Converter::Int.apply(&json!("  7 ")), Some(json!(7)));
        assert_eq!(Converter::Str.apply(&json!(1.5)), Some(json!("1.5")));
        assert_eq!(Converter::Str.apply(&json!(null)), Some(json!("None")));
        assert_eq!(Converter::Str.apply(&json!([1, 2])), Some(json!("[1, 2]")));
        // s_to_ms 只认数值，字符串一律 SKIP（与 int 转换器不同）。
        assert_eq!(Converter::SToMs.apply(&json!("3")), None);
        assert_eq!(Converter::MsToS.apply(&json!(1)), Some(json!(0.001)));
        assert_eq!(
            Converter::SandboxFlag.apply(&json!(0)),
            Some(json!("default"))
        );
        assert_eq!(
            Converter::SandboxFlag.apply(&json!("x")),
            Some(json!("dangerously-disable"))
        );
        assert_eq!(
            Converter::SandboxUnflag.apply(&json!(1)),
            Some(json!(false))
        );
        assert_eq!(Converter::Bool.apply(&json!(1)), None);
        assert_eq!(Converter::Str.apply(&json!(150)), Some(json!("150")));
        assert_eq!(Converter::Str.apply(&json!("x")), Some(json!("x")));
        assert_eq!(Converter::Str.apply(&json!(true)), Some(json!("True")));
        assert_eq!(Converter::SToMs.apply(&json!(1.5)), Some(json!(1500)));
        assert_eq!(Converter::SToMs.apply(&json!(true)), None);
        assert_eq!(Converter::MsToS.apply(&json!(5000)), Some(json!(5.0)));
        assert_eq!(
            Converter::SandboxFlag.apply(&json!(true)),
            Some(json!("dangerously-disable"))
        );
        assert_eq!(
            Converter::SandboxFlag.apply(&json!("")),
            Some(json!("default"))
        );
        assert_eq!(
            Converter::SandboxUnflag.apply(&json!("dangerously-disable")),
            Some(json!(true))
        );
        assert_eq!(Converter::Bool.apply(&json!("TRUE")), Some(json!(true)));
        assert_eq!(Converter::Bool.apply(&json!("yes")), None);
        assert_eq!(Converter::from_name("int"), Some(Converter::Int));
        assert_eq!(Converter::from_name("eval"), None);
    }

    #[test]
    fn parse_applies_defaults_and_decoders() {
        let dialect = shell_dialect();
        let (op, canonical) = dialect
            .parse(
                "Bash",
                &json!({"timeout": 5, "dangerouslyDisableSandbox": true, "extra": 1}),
            )
            .unwrap();
        assert_eq!(op, CanonicalOp::SHELL_EXEC);
        // command 缺席 -> read_default ""；extras=ignore 时 extra 被丢弃。
        assert_eq!(
            canonical,
            json!({"command": "", "timeout_ms": 5,
                   "sandbox_policy": "dangerously-disable"})
        );
    }

    #[test]
    fn nulls_and_drop_native_keys_never_reach_the_binding() {
        let dialect = ToolDialect::new(
            "test",
            "test",
            vec![OpBinding::new(
                CanonicalOp::SHELL_EXEC,
                "Bash",
                vec![FieldMap::new("command")],
            )
            .extras(Extras::Fallback)],
        )
        .drop_native(["variant"]);
        // null 与 drop_native 键都不触发 fallback 守卫。
        let (_, canonical) = dialect
            .parse(
                "Bash",
                &json!({"command": "ls", "variant": "chat", "opt": null}),
            )
            .unwrap();
        assert_eq!(canonical, json!({"command": "ls"}));
        // 真正的表外字段则整体退回。
        assert!(dialect
            .parse("Bash", &json!({"command": "ls", "flag": "-i"}))
            .is_none());
    }

    #[test]
    fn strict_input_decides_what_happens_to_non_dict_inputs() {
        let lenient = shell_dialect();
        assert_eq!(
            lenient.parse("Bash", &json!("ls -la")),
            Some((CanonicalOp::SHELL_EXEC, json!("ls -la")))
        );
        let strict = shell_dialect().strict_input(true);
        assert!(strict.parse("Bash", &json!("ls -la")).is_none());
        // 未知工具名一律 None。
        assert!(lenient.parse("Nope", &json!({})).is_none());
    }

    #[test]
    fn render_inlines_workdir_and_flags_it_transformed() {
        let dialect = shell_dialect();
        let canonical = json!({"command": "ls", "workdir": "/a b", "sandbox_policy": "default"});
        let (name, native) = dialect.render(CanonicalOp::SHELL_EXEC, &canonical).unwrap();
        assert_eq!(name, "Bash");
        assert_eq!(native.get("command"), Some(&json!("cd '/a b' && ls")));
        assert_eq!(native.get("dangerouslyDisableSandbox"), Some(&json!(false)));
        let flags = dialect
            .binding_for(CanonicalOp::SHELL_EXEC)
            .unwrap()
            .render_flags_hook()
            .unwrap()(&map(canonical), &native);
        assert_eq!(flags.fidelity, Some(Fidelity::Transformed));
        assert_eq!(flags.reason_codes, ["workdir_inlined"]);
    }

    #[test]
    fn supported_fields_include_encode_post_fields_only_when_writable() {
        let dialect = shell_dialect();
        let supported = dialect.supported_fields(CanonicalOp::SHELL_EXEC);
        assert!(supported.contains("workdir"));
        assert!(supported.contains("command"));
        assert!(dialect.supported_fields("fs.read").is_empty());
    }

    #[test]
    fn readonly_bindings_only_serve_the_read_path() {
        let dialect = ToolDialect::new(
            "test",
            "test",
            vec![
                OpBinding::new(
                    CanonicalOp::FS_READ,
                    "read_file",
                    vec![FieldMap::new("file_path")],
                ),
                OpBinding::new(
                    CanonicalOp::FS_READ,
                    "Read",
                    vec![FieldMap::new("file_path")],
                )
                .readonly(),
            ],
        );
        assert_eq!(dialect.op_for("Read"), Some(CanonicalOp::FS_READ));
        // 写端只认第一个非 readonly 绑定。
        let (name, _) = dialect
            .render(CanonicalOp::FS_READ, &json!({"file_path": "/a"}))
            .unwrap();
        assert_eq!(name, "read_file");
        assert_eq!(dialect.write_ops(), ["fs.read".to_string()].into());
    }

    #[test]
    fn shell_quote_matches_shlex() {
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("/usr/local"), "/usr/local");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r#"'it'"'"'s'"#);
        assert_eq!(shell_quote("中文"), "'中文'");
    }

    #[test]
    fn the_registry_is_a_static_table() {
        static PROBE: LazyLock<ToolDialect> = LazyLock::new(shell_dialect);
        assert!(get_dialect("wp-b2-probe").is_none());
        register_dialect("wp-b2-probe", &PROBE);
        assert_eq!(get_dialect("wp-b2-probe").unwrap().adapter(), "test");
        assert!(registered_dialects().contains(&"wp-b2-probe"));
    }
}
