//! 行分隔会话存储的共享扫描机制。
//!
//! 语义事实源：`engine/adapters/shared/scanner.py` + `engine/sessions/topology.py`。
//!
//! 分层说明：Python 的 scanner 反向 import 了 `sessions.topology.session_roots`
//! 与 `sessions.scan_progress.TRACKER`。Rust 的 `adapters` 不得引用 `sessions`
//! （见 `adapters/mod.rs`），因此：
//! - [`session_roots`] 树装配**下沉到本模块**，由 `sessions` 复用；
//! - 扫描进度改成注册式回调 [`install_scan_progress`]，`sessions::scan_progress`
//!   在装配时把自己的 TRACKER 注册进来；没注册时全部上报是空操作
//!   （与 Python「未处于扫描中的上报一律忽略」同义）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};

use rayon::prelude::*;
use serde_json::{Map, Value};

use crate::adapters::contracts::{ScanCache, ScanRow};
use crate::errors::{DomainError, DomainResult};
use crate::jsonutil::FileStat;

/// 超过这个文件数才起线程池（对齐 Python 的 `_PARALLEL_SCAN_THRESHOLD`）。
const PARALLEL_SCAN_THRESHOLD: usize = 16;

fn scan_workers() -> usize {
    std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(4)
        .min(8)
}

static SCAN_POOL: LazyLock<rayon::ThreadPool> = LazyLock::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(scan_workers())
        .thread_name(|index| format!("ferry-scan-{index}"))
        .build()
        .expect("扫描线程池构建失败")
});

/// 扫描进度上报口。由 `sessions::scan_progress` 实现并注册。
pub trait ScanProgress: Send + Sync {
    fn set_total(&self, total: usize);
    fn advance(&self, count: usize);
}

static PROGRESS: OnceLock<&'static dyn ScanProgress> = OnceLock::new();

/// 注册全局进度上报口；只有第一次调用生效。
pub fn install_scan_progress(sink: &'static dyn ScanProgress) {
    let _ = PROGRESS.set(sink);
}

fn progress() -> Option<&'static dyn ScanProgress> {
    PROGRESS.get().copied()
}

/// 上报本轮扫描的总量。给不走 [`scan_jsonl`] 的 adapter（grok 是目录型，自己
/// 递归 rglob）用；没注册进度口时是空操作。
pub fn report_scan_total(total: usize) {
    if let Some(sink) = progress() {
        sink.set_total(total);
    }
}

/// 上报扫描进度。语义同 [`report_scan_total`]。
pub fn report_scan_advance(count: usize) {
    if let Some(sink) = progress() {
        sink.advance(count);
    }
}

/// 压平空白后按字符截断，超长补省略号。
pub fn clip_text(text: &str, size: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut clipped: String = joined.chars().take(size).collect();
    if joined.chars().count() > size {
        clipped.push('…');
    }
    clipped
}

/// `clip_text` 的默认长度（Python 的 `size=80`）。
pub fn clip_text_default(text: &str) -> String {
    clip_text(text, 80)
}

/// 把文件 stat 折成稳定的修订标记；实现在 `jsonutil`，这里只做路径转字符串。
pub fn stat_digest(path: &Path, stat: &FileStat) -> String {
    crate::jsonutil::stat_digest(&path.to_string_lossy(), stat)
}

/// Agent 检索阶段的 O(1) 修订标记；深度校验留给写入链路。
pub fn path_stat_fingerprint(reference: &str) -> io::Result<String> {
    let path = fs::canonicalize(reference)?;
    let stat = FileStat::from_metadata(&fs::metadata(&path)?);
    Ok(stat_digest(&path, &stat))
}

/// 逐行读 JSONL。
///
/// 不用「整读再 split」：大会话的峰值内存是文件体积的两倍。分行规则与 Python
/// 文本模式的 universal newlines 一致（`\n` / `\r\n` / `\r` 都是记录边界，
/// U+0085 / U+2028 / U+2029 原样穿透），行尾终止符不进结果。
pub fn iter_lines(path: &Path) -> io::Result<Lines<BufReader<fs::File>>> {
    Ok(Lines::new(BufReader::new(fs::File::open(path)?)))
}

/// [`iter_lines`] 的迭代器实现。
pub struct Lines<R: BufRead> {
    reader: R,
    /// 上一行以 `\r` 收尾，紧跟的 `\n` 属于同一个终止符。
    skip_lf: bool,
}

impl<R: BufRead> Lines<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            skip_lf: false,
        }
    }
}

impl<R: BufRead> Iterator for Lines<R> {
    type Item = io::Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line: Vec<u8> = Vec::new();
        let mut started = false;
        loop {
            let outcome = {
                let buffer = match self.reader.fill_buf() {
                    Ok(buffer) => buffer,
                    Err(error) => return Some(Err(error)),
                };
                if buffer.is_empty() {
                    if !started {
                        return None;
                    }
                    return Some(decode(line));
                }
                started = true;
                if self.skip_lf {
                    self.skip_lf = false;
                    if buffer[0] == b'\n' {
                        (1usize, None)
                    } else {
                        (0usize, None)
                    }
                } else {
                    match buffer
                        .iter()
                        .position(|byte| *byte == b'\n' || *byte == b'\r')
                    {
                        Some(index) => {
                            line.extend_from_slice(&buffer[..index]);
                            (index + 1, Some(buffer[index]))
                        }
                        None => {
                            line.extend_from_slice(buffer);
                            (buffer.len(), None)
                        }
                    }
                }
            };
            let (used, terminator) = outcome;
            self.reader.consume(used);
            match terminator {
                Some(b'\r') => {
                    self.skip_lf = true;
                    return Some(decode(line));
                }
                Some(_) => return Some(decode(line)),
                None => {}
            }
        }
    }
}

fn decode(line: Vec<u8>) -> io::Result<String> {
    String::from_utf8(line).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// 只按 JSON Lines 规定的 LF 分隔记录。
///
/// **不是** `str::lines()`：后者会吃掉 `\r`，也不保留结尾空段；
/// U+0085 / U+2028 / U+2029 一律当普通字符穿透。
pub fn split_jsonl_lines(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

/// `parse` 回调的结果。
pub enum ScanOutcome {
    /// 解析出扫描行；空 map 等价 Python 的 falsy meta（写缓存但不进结果）。
    Row(ScanRow),
    /// 对齐 Python `except (json.JSONDecodeError, OSError)`：跳过且**不写缓存**。
    Skipped,
}

/// 扫描带缓存的 JSONL 文件；adapter 只需实现自己的记录 schema。
///
/// 结果保序（与串行扫描逐条一致），最后交给 [`session_roots`] 装配成树。
pub fn scan_jsonl(
    pattern: &str,
    cache: &dyn ScanCache,
    parse: &(dyn Fn(&Path, &FileStat) -> DomainResult<ScanOutcome> + Sync),
) -> DomainResult<Vec<ScanRow>> {
    let filenames: Vec<PathBuf> = match glob::glob(pattern) {
        Ok(paths) => paths.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    };
    // 进度上报只在 RPC scan 期间生效，其他入口（如内容索引预热）是空操作。
    if let Some(sink) = progress() {
        sink.set_total(filenames.len());
    }
    // JSONL 解析是冷启动最耗时的一段，文件之间互不依赖；结果按输入顺序回收，
    // 输出与串行时一致。
    let parsed: Vec<DomainResult<Option<ScanRow>>> = if filenames.len() < PARALLEL_SCAN_THRESHOLD {
        filenames
            .iter()
            .map(|path| scan_one(path, cache, parse))
            .collect()
    } else {
        SCAN_POOL.install(|| {
            filenames
                .par_iter()
                .map(|path| scan_one(path, cache, parse))
                .collect()
        })
    };
    let mut rows = Vec::with_capacity(parsed.len());
    for item in parsed {
        if let Some(row) = item? {
            rows.push(row);
        }
    }
    session_roots(rows)
}

fn scan_one(
    path: &Path,
    cache: &dyn ScanCache,
    parse: &(dyn Fn(&Path, &FileStat) -> DomainResult<ScanOutcome> + Sync),
) -> DomainResult<Option<ScanRow>> {
    if let Some(sink) = progress() {
        sink.advance(1);
    }
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(None);
    };
    let stat = FileStat::from_metadata(&metadata);
    if let Some(cached) = cache.get(path, &stat) {
        return Ok(cached.filter(|row| !row.is_empty()));
    }
    let meta = match parse(path, &stat)? {
        ScanOutcome::Skipped => return Ok(None),
        ScanOutcome::Row(row) => row,
    };
    let stored = if meta.is_empty() { None } else { Some(meta) };
    cache.put(path, &stat, stored.clone());
    Ok(stored)
}

// ---------------------------------------------------------------------------
// 会话树规则（Python: engine/sessions/topology.py）
// ---------------------------------------------------------------------------

struct TreeNode {
    row: ScanRow,
    children: Vec<usize>,
    own_count: Num,
    own_size: Num,
    own_updated: Num,
    count: Num,
    size: Num,
    updated: Num,
    tree_count: i64,
}

/// Python 数值的最小模型：`int` 与 `float` 是两种类型，加法与 `max` 都保留类型。
///
/// 扫描行的 `count` / `size` / `updated` 由各 adapter 产出，**允许是 float**
/// （grok 的 `num_chat_messages`、opencode 的时间戳都可能落成浮点）。截断成
/// `i64` 会让树汇总值与 Python 分叉，而这几个字段直接进 `scan` 的 wire 出参。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    const ZERO: Self = Self::Int(0);

    /// Python 侧这些值只会参与 `+` 与 `max`；非数值一律在那里抛 `TypeError`。
    /// 注意 `bool` 在 Python 里是 `int` 的子类，参与算术合法。
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(flag) => Some(Self::Int(i64::from(*flag))),
            Value::Number(number) => number
                .as_i64()
                .map(Self::Int)
                .or_else(|| number.as_f64().map(Self::Float)),
            _ => None,
        }
    }

    fn as_f64(self) -> f64 {
        match self {
            Self::Int(value) => value as f64,
            Self::Float(value) => value,
        }
    }

    fn add(self, other: Self) -> Self {
        match (self, other) {
            (Self::Int(left), Self::Int(right)) => Self::Int(left.saturating_add(right)),
            _ => Self::Float(self.as_f64() + other.as_f64()),
        }
    }

    /// `max(a, b)`：并列取**前者**（Python `max` 的语义）。
    fn max(self, other: Self) -> Self {
        if other.as_f64() > self.as_f64() {
            other
        } else {
            self
        }
    }

    fn to_value(self) -> Value {
        match self {
            Self::Int(value) => Value::from(value),
            Self::Float(value) => Value::from(value),
        }
    }
}

/// `own_*` 的取值：`source.get(own_key, source.get(fallback_key, 0))`。
///
/// **只有键缺失才回落**——显式 `null` 在 Python 里会原样留下并在后续 `+` 上
/// 抛 `TypeError`（未捕获异常 → `internal.unexpected`），不能悄悄当成 0。
fn own_number(source: &ScanRow, own_key: &str, fallback_key: &str) -> DomainResult<Num> {
    let Some(raw) = source.get(own_key).or_else(|| source.get(fallback_key)) else {
        return Ok(Num::ZERO);
    };
    Num::from_value(raw).ok_or_else(|| {
        DomainError::internal(format!(
            "扫描行的 {own_key}/{fallback_key} 必须是数值: {raw}"
        ))
    })
}

/// 会话树的节点键。
///
/// Python 用 `nodes[node["id"]]` 直接拿原值当 dict 键：`None`、数字、字符串都是
/// 合法且**互不相等**的键（grok 的扫描行就允许 `id: null`）。这里用值的 JSON
/// 文本做键，保住「不同类型不同键」这一条；塌成空串会把多条无 id 行合并掉。
fn id_key(value: &Value) -> String {
    value.to_string()
}

/// `node["id"]`：键缺失等价 Python 的 `KeyError`，整批扫描失败而不是静默丢行。
fn row_id(row: &ScanRow) -> DomainResult<&Value> {
    row.get("id")
        .ok_or_else(|| DomainError::internal("扫描行缺少 id"))
}

/// 把扁平扫描行装配成父子树：补 own_* / root_id / child_count / tree_count，
/// 逐层汇总 count / size / updated，并按 updated 降序排序。
pub fn session_roots(rows: Vec<ScanRow>) -> DomainResult<Vec<ScanRow>> {
    let mut nodes: Vec<TreeNode> = Vec::with_capacity(rows.len());
    // Python 的 dict 按 id 去重：后来者覆盖值但保留首次出现的位置。
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<usize> = Vec::with_capacity(rows.len());
    for source in rows {
        let own_count = own_number(&source, "own_count", "count")?;
        let own_size = own_number(&source, "own_size", "size")?;
        let own_updated = own_number(&source, "own_updated", "updated")?;
        let id = id_key(row_id(&source)?);
        let mut row = source;
        row.insert("children".into(), Value::Array(Vec::new()));
        row.insert("own_count".into(), own_count.to_value());
        row.insert("own_size".into(), own_size.to_value());
        row.insert("own_updated".into(), own_updated.to_value());
        let node = TreeNode {
            row,
            children: Vec::new(),
            own_count,
            own_size,
            own_updated,
            count: Num::ZERO,
            size: Num::ZERO,
            updated: Num::ZERO,
            tree_count: 0,
        };
        match by_id.get(&id) {
            Some(index) => nodes[*index] = node,
            None => {
                nodes.push(node);
                by_id.insert(id, nodes.len() - 1);
                order.push(nodes.len() - 1);
            }
        }
    }

    let parent_of = |nodes: &[TreeNode], index: usize| -> Option<usize> {
        // `nodes.get(node.get("parent_id"))`：缺键即 `None` 键，与显式 null 同义。
        let parent = nodes[index].row.get("parent_id").unwrap_or(&Value::Null);
        by_id.get(&id_key(parent)).copied()
    };

    // 环检测：沿 parent 链上溯，撞到自己走过的 id 就把环上的节点全部标记。
    let mut cyclic: HashSet<usize> = HashSet::new();
    for index in &order {
        let mut cursor = Some(*index);
        let mut path: Vec<usize> = Vec::new();
        while let Some(current) = cursor {
            if path.contains(&current) {
                break;
            }
            path.push(current);
            cursor = parent_of(&nodes, current);
        }
        if let Some(current) = cursor {
            if let Some(position) = path.iter().position(|item| *item == current) {
                cyclic.extend(path[position..].iter().copied());
            }
        }
    }

    let mut roots: Vec<usize> = Vec::new();
    for index in &order {
        let parent = if cyclic.contains(index) {
            None
        } else {
            parent_of(&nodes, *index)
        };
        match parent {
            Some(parent) if parent != *index => nodes[parent].children.push(*index),
            _ => {
                nodes[*index].row.insert("parent_id".into(), Value::Null);
                roots.push(*index);
            }
        }
    }

    let mut visiting: HashSet<usize> = HashSet::new();
    for root in &roots {
        let root_id = nodes[*root].row.get("id").cloned().unwrap_or(Value::Null);
        summarize(&mut nodes, *root, &root_id, &mut visiting);
    }
    // 降序稳定排序；`updated` 可能是 float，不能走整数 key 排序。
    roots.sort_by(|left, right| descending(nodes[*right].updated, nodes[*left].updated));
    Ok(roots
        .into_iter()
        .map(|index| materialize(&mut nodes, index))
        .collect())
}

/// `sort(key=..., reverse=True)` 的比较子：NaN 视作相等（Python 的排序对 NaN
/// 也不给保证），保证是全序，`sort_by` 才不会 panic。
fn descending(left: Num, right: Num) -> std::cmp::Ordering {
    left.as_f64()
        .partial_cmp(&right.as_f64())
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn summarize(nodes: &mut [TreeNode], index: usize, root_id: &Value, visiting: &mut HashSet<usize>) {
    if visiting.contains(&index) {
        nodes[index].children.clear();
    }
    visiting.insert(index);
    nodes[index].row.insert("root_id".into(), root_id.clone());
    let children = nodes[index].children.clone();
    for child in &children {
        summarize(nodes, *child, root_id, visiting);
    }
    visiting.remove(&index);

    let mut children = nodes[index].children.clone();
    // Python 的 `sort(key=..., reverse=True)` 是稳定的：等值项保持原序。
    children.sort_by(|left, right| descending(nodes[*right].updated, nodes[*left].updated));
    let child_count = children.len() as i64;
    let tree_count = 1 + children
        .iter()
        .map(|child| nodes[*child].tree_count)
        .sum::<i64>();
    // `sum()` 从 int 0 起算；任一项是 float 则结果是 float（Python 语义）。
    let count = children
        .iter()
        .fold(Num::ZERO, |total, child| total.add(nodes[*child].count));
    let count = nodes[index].own_count.add(count);
    let size = children
        .iter()
        .fold(Num::ZERO, |total, child| total.add(nodes[*child].size));
    let size = nodes[index].own_size.add(size);
    let updated = children
        .iter()
        .fold(nodes[index].own_updated, |best, child| {
            best.max(nodes[*child].updated)
        });

    let node = &mut nodes[index];
    node.children = children;
    node.tree_count = tree_count;
    node.count = count;
    node.size = size;
    node.updated = updated;
    node.row
        .insert("child_count".into(), Value::from(child_count));
    node.row
        .insert("tree_count".into(), Value::from(tree_count));
    node.row.insert("count".into(), count.to_value());
    node.row.insert("size".into(), size.to_value());
    node.row.insert("updated".into(), updated.to_value());
}

fn materialize(nodes: &mut [TreeNode], index: usize) -> ScanRow {
    let children = std::mem::take(&mut nodes[index].children);
    let mut row = std::mem::take(&mut nodes[index].row);
    let items: Vec<Value> = children
        .into_iter()
        .map(|child| Value::Object(materialize(nodes, child)))
        .collect();
    // key 已存在，insert 只换值不换位置（serde_json preserve_order）。
    row.insert("children".into(), Value::Array(items));
    row
}

// ---------------------------------------------------------------------------
// 扫描行的 token 与时间归一化（Python: engine/sessions/usage.py）
// ---------------------------------------------------------------------------
//
// 这几个纯函数在 Python 侧由 `adapters/**/scanner.py` 反向 import
// `sessions.usage`；Rust 禁止 `adapters → sessions`（方案 §1.1），所以实现落在
// 这里，由 `sessions::usage` re-export。

/// 归一化 token 桶的四个键，顺序即 DTO 里的键序。
pub const TOKEN_KEYS: [&str; 4] = ["input", "output", "cache_read", "cache_write"];

/// 归一化后的 token 计数。
///
/// 三个工具的原始字段口径不同，统一成 `{input, output, cache_read, cache_write}`；
/// `input` 只计未命中缓存的输入（缓存读取单独放 `cache_read`），便于前端按
/// models.dev 单价分档估算成本。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl Tokens {
    pub fn get(&self, key: &str) -> i64 {
        match key {
            "input" => self.input,
            "output" => self.output,
            "cache_read" => self.cache_read,
            _ => self.cache_write,
        }
    }

    pub fn total(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn to_value(self) -> Value {
        let mut payload = Map::new();
        for key in TOKEN_KEYS {
            payload.insert(key.into(), Value::from(self.get(key)));
        }
        Value::Object(payload)
    }

    /// 从原生 JSON 桶读数，对齐 `int(other.get(key) or 0)`：缺键/null/false → 0，
    /// 浮点向零截断。字符串数字在 Python 里会被 `int()` 接受，但真实扫描行里
    /// 这四个字段恒为数值，这里按 0 处理以免把非法值当成计数。
    pub fn from_value(value: &Value) -> Self {
        let read = |key: &str| -> i64 {
            match value.get(key) {
                Some(Value::Number(number)) => number
                    .as_i64()
                    .or_else(|| number.as_f64().map(|float| float.trunc() as i64))
                    .unwrap_or(0),
                Some(Value::Bool(true)) => 1,
                _ => 0,
            }
        };
        Self {
            input: read("input"),
            output: read("output"),
            cache_read: read("cache_read"),
            cache_write: read("cache_write"),
        }
    }
}

/// `empty_tokens()`。
pub fn empty_tokens() -> Tokens {
    Tokens::default()
}

/// `add_tokens(acc, other)`。
pub fn add_tokens(accumulator: &mut Tokens, other: &Tokens) {
    accumulator.input += other.input;
    accumulator.output += other.output;
    accumulator.cache_read += other.cache_read;
    accumulator.cache_write += other.cache_write;
}

/// `has_tokens(tokens)`：四个键里任意一个非零。
pub fn has_tokens(tokens: &Tokens) -> bool {
    TOKEN_KEYS.iter().any(|key| tokens.get(key) != 0)
}

/// 出现 token 最多的模型作为该会话的代表模型。
///
/// 入参保持插入序（Python 是 dict）：并列时**先出现**的胜出，因为 Python 的
/// `max()` 只在严格大于时才换人。
pub fn dominant_model(by_model: &[(String, Tokens)]) -> String {
    by_model
        .iter()
        .fold(None::<(&str, i64)>, |best, (model, tokens)| {
            let total = tokens.total();
            match best {
                Some((_, best_total)) if best_total >= total => best,
                _ => Some((model.as_str(), total)),
            }
        })
        .map(|(model, _)| model.to_string())
        .unwrap_or_default()
}

/// ISO8601（带 Z）转毫秒时间戳；已是数字则原样返回；解析失败返回 `None`。
///
/// naive datetime 当 UTC（对齐 `replace(tzinfo=timezone.utc)`）。
pub fn iso_ms(value: &Value) -> Option<i64> {
    match value {
        Value::Null => None,
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64)),
        other => {
            let text = match other {
                Value::String(text) => text.clone(),
                _ => other.to_string(),
            };
            parse_iso8601_ms(&text)
        }
    }
}

/// `datetime.fromisoformat(text.replace("Z", "+00:00"))` 的等价实现。
///
/// 覆盖 Python 3.11+ 接受的扩展格式与基本格式：日期 `YYYY-MM-DD` / `YYYYMMDD`，
/// 可选的任意单字符日期时间分隔符，时间 `HH[:MM[:SS[.f{1,6}]]]`（含无冒号形式），
/// 时区 `Z` / `±HH[:MM[:SS]]`。naive 值按 UTC 处理。
pub fn parse_iso8601_ms(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() < 8 {
        return None;
    }
    let (date_part, rest) = split_date(text)?;
    let (year, month, day) = date_part;
    let mut millis = civil_days(year, month, day)? * 86_400_000;
    if rest.is_empty() {
        return Some(millis);
    }
    // 第一个字符是日期/时间分隔符，Python 接受任意单字符。
    let mut time_text = &rest[rest.chars().next()?.len_utf8()..];
    let mut offset_ms = 0i64;
    if let Some(position) = time_text
        .rfind(['+', '-'])
        .filter(|position| *position > 0)
        .or_else(|| {
            time_text
                .find(['Z', 'z'])
                .filter(|position| *position + 1 == time_text.len())
        })
    {
        let (head, tail) = time_text.split_at(position);
        offset_ms = parse_offset(tail)?;
        time_text = head;
    }
    millis += parse_time_ms(time_text)?;
    Some(millis - offset_ms)
}

#[allow(clippy::type_complexity)]
fn split_date(text: &str) -> Option<((i64, u32, u32), &str)> {
    let bytes = text.as_bytes();
    if bytes.len() >= 10 && bytes[4] == b'-' && bytes[7] == b'-' {
        let year = text[0..4].parse().ok()?;
        let month = text[5..7].parse().ok()?;
        let day = text[8..10].parse().ok()?;
        return Some(((year, month, day), &text[10..]));
    }
    if bytes.len() >= 8 && bytes[0..8].iter().all(u8::is_ascii_digit) {
        let year = text[0..4].parse().ok()?;
        let month = text[4..6].parse().ok()?;
        let day = text[6..8].parse().ok()?;
        return Some(((year, month, day), &text[8..]));
    }
    None
}

fn parse_offset(text: &str) -> Option<i64> {
    if matches!(text, "Z" | "z") {
        return Some(0);
    }
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    Some(sign * parse_time_ms(&text[1..])?)
}

/// `HH`、`HH:MM`、`HH:MM:SS[.f{1,6}]` 与对应的无冒号形式。
fn parse_time_ms(text: &str) -> Option<i64> {
    if text.is_empty() {
        return Some(0);
    }
    let (main, fraction) = match text.split_once('.') {
        Some((main, fraction)) => (main, Some(fraction)),
        None => (text, None),
    };
    let digits: Vec<&str> = if main.contains(':') {
        main.split(':').collect()
    } else {
        if !main.as_bytes().iter().all(u8::is_ascii_digit) || main.len() % 2 != 0 {
            return None;
        }
        (0..main.len() / 2)
            .map(|group| &main[group * 2..group * 2 + 2])
            .collect()
    };
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    let mut total = 0i64;
    for (position, chunk) in digits.iter().enumerate() {
        let value: i64 = chunk.parse().ok()?;
        total += value * [3_600_000i64, 60_000, 1_000][position];
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty()
            || fraction.len() > 6
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        // 只取毫秒精度：更细的微秒在 epoch 毫秒里本就落不下。
        let mut padded = fraction.to_string();
        while padded.len() < 3 {
            padded.push('0');
        }
        total += padded[..3].parse::<i64>().ok()?;
    }
    Some(total)
}

/// 民用日期 → 自 1970-01-01 起的天数（Howard Hinnant `days_from_civil`）。
fn civil_days(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    fn row(value: Value) -> ScanRow {
        value.as_object().cloned().unwrap()
    }

    #[derive(Default)]
    struct MemoryCache {
        entries: Mutex<HashMap<PathBuf, Option<ScanRow>>>,
        hits: Mutex<usize>,
    }

    impl ScanCache for MemoryCache {
        fn get(&self, path: &Path, _stat: &FileStat) -> Option<Option<ScanRow>> {
            let found = self.entries.lock().unwrap().get(path).cloned();
            if found.is_some() {
                *self.hits.lock().unwrap() += 1;
            }
            found
        }

        fn put(&self, path: &Path, _stat: &FileStat, meta: Option<ScanRow>) {
            self.entries
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), meta);
        }

        fn get_digest(&self, _path: &Path, _stat: &FileStat) -> Option<String> {
            None
        }

        fn put_digest(&self, _path: &Path, _stat: &FileStat, _digest: &str) {}

        fn flush(&self) {}
    }

    #[test]
    fn split_jsonl_lines_only_honours_line_feed() {
        // U+0085 / U+2028 / U+2029 必须原样穿透，否则会把一条记录切成两条。
        let text = "a\u{85}b\u{2028}c\u{2029}d\ne";
        assert_eq!(split_jsonl_lines(text), ["a\u{85}b\u{2028}c\u{2029}d", "e"]);
        // 结尾空段保留（Python `str.split("\n")` 的语义）。
        assert_eq!(split_jsonl_lines("a\n"), ["a", ""]);
        assert_eq!(split_jsonl_lines(""), [""]);
        // \r 不是分隔符。
        assert_eq!(split_jsonl_lines("a\r\nb"), ["a\r", "b"]);
    }

    #[test]
    fn iter_lines_follows_python_universal_newlines() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("a.jsonl");
        fs::write(&path, "a\r\nb\rc\nd\u{2028}e\n").unwrap();
        let lines: Vec<String> = iter_lines(&path).unwrap().map(Result::unwrap).collect();
        assert_eq!(lines, ["a", "b", "c", "d\u{2028}e"]);

        // 没有结尾换行时最后一行照样产出，且不会多出空行。
        fs::write(&path, "x").unwrap();
        let lines: Vec<String> = iter_lines(&path).unwrap().map(Result::unwrap).collect();
        assert_eq!(lines, ["x"]);

        fs::write(&path, "").unwrap();
        assert_eq!(iter_lines(&path).unwrap().count(), 0);

        fs::write(&path, "\n\n").unwrap();
        let lines: Vec<String> = iter_lines(&path).unwrap().map(Result::unwrap).collect();
        assert_eq!(lines, ["", ""]);
    }

    #[test]
    fn clip_text_collapses_whitespace_and_appends_ellipsis() {
        assert_eq!(clip_text("  a \n b  ", 80), "a b");
        assert_eq!(clip_text("中文中文", 2), "中文…");
        assert_eq!(clip_text("ab", 2), "ab");
    }

    #[test]
    fn scan_jsonl_is_ordered_and_cache_aware() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..3 {
            fs::write(root.path().join(format!("s{index}.jsonl")), "{}").unwrap();
        }
        let cache = MemoryCache::default();
        let pattern = format!("{}/*.jsonl", root.path().display());
        let parse = |path: &Path, _stat: &FileStat| -> DomainResult<ScanOutcome> {
            let id = path.file_stem().unwrap().to_string_lossy().into_owned();
            Ok(ScanOutcome::Row(row(
                json!({"id": id, "updated": 1, "count": 1, "size": 10}),
            )))
        };
        let rows = scan_jsonl(&pattern, &cache, &parse).unwrap();
        let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["s0", "s1", "s2"]);
        // 第二次全部命中缓存。
        let rows = scan_jsonl(&pattern, &cache, &parse).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(*cache.hits.lock().unwrap(), 3);
    }

    #[test]
    fn skipped_files_are_not_cached() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("bad.jsonl"), "not json").unwrap();
        let cache = MemoryCache::default();
        let pattern = format!("{}/*.jsonl", root.path().display());
        let skip = |_path: &Path, _stat: &FileStat| -> DomainResult<ScanOutcome> {
            Ok(ScanOutcome::Skipped)
        };
        let rows = scan_jsonl(&pattern, &cache, &skip).unwrap();
        assert!(rows.is_empty());
        assert!(cache.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn empty_rows_are_cached_as_not_a_session() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("empty.jsonl"), "{}").unwrap();
        let cache = MemoryCache::default();
        let pattern = format!("{}/*.jsonl", root.path().display());
        let empty = |_path: &Path, _stat: &FileStat| -> DomainResult<ScanOutcome> {
            Ok(ScanOutcome::Row(ScanRow::new()))
        };
        let rows = scan_jsonl(&pattern, &cache, &empty).unwrap();
        assert!(rows.is_empty());
        assert_eq!(cache.entries.lock().unwrap().len(), 1);
    }

    #[test]
    fn session_roots_builds_the_tree_and_aggregates() {
        let rows = vec![
            row(json!({"id": "root", "updated": 10, "count": 1, "size": 100})),
            row(json!({"id": "child", "parent_id": "root", "updated": 20,
                       "count": 2, "size": 200})),
            row(json!({"id": "other", "updated": 5, "count": 1, "size": 50})),
        ];
        let roots = session_roots(rows).unwrap();
        let ids: Vec<&str> = roots
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        // 根按 updated 降序：root 汇总后 updated=20 > other 的 5。
        assert_eq!(ids, ["root", "other"]);
        let first = &roots[0];
        assert_eq!(first["count"], json!(3));
        assert_eq!(first["size"], json!(300));
        assert_eq!(first["updated"], json!(20));
        assert_eq!(first["own_count"], json!(1));
        assert_eq!(first["own_updated"], json!(10));
        assert_eq!(first["child_count"], json!(1));
        assert_eq!(first["tree_count"], json!(2));
        assert_eq!(first["root_id"], json!("root"));
        assert_eq!(first["parent_id"], Value::Null);
        let children = first["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["root_id"], json!("root"));
        assert_eq!(children[0]["parent_id"], json!("root"));
    }

    #[test]
    fn cyclic_parents_are_promoted_to_roots() {
        let rows = vec![
            row(json!({"id": "a", "parent_id": "b", "updated": 1})),
            row(json!({"id": "b", "parent_id": "a", "updated": 2})),
            row(json!({"id": "self", "parent_id": "self", "updated": 3})),
        ];
        let roots = session_roots(rows).unwrap();
        let mut ids: Vec<&str> = roots
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, ["a", "b", "self"]);
        for root in &roots {
            assert_eq!(root["parent_id"], Value::Null);
            assert_eq!(root["tree_count"], json!(1));
        }
    }

    #[test]
    fn missing_parents_fall_back_to_root() {
        let rows = vec![row(
            json!({"id": "orphan", "parent_id": "gone", "updated": 1}),
        )];
        let roots = session_roots(rows).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["parent_id"], Value::Null);
        assert_eq!(roots[0]["root_id"], json!("orphan"));
    }

    #[test]
    fn float_counters_are_not_truncated() {
        // Python 侧 `count`/`size`/`updated` 是原样搬运的数值，可能是浮点；
        // 截断成整数会让树汇总与 Python 分叉，而这些字段直接进 scan 的出参。
        let rows = vec![
            row(json!({"id": "root", "updated": 10.5, "count": 1.5, "size": 2})),
            row(json!({"id": "child", "parent_id": "root", "updated": 20.25,
                       "count": 2, "size": 3.75})),
        ];
        let roots = session_roots(rows).unwrap();
        assert_eq!(roots[0]["own_count"], json!(1.5));
        assert_eq!(roots[0]["count"], json!(3.5));
        assert_eq!(roots[0]["size"], json!(5.75));
        assert_eq!(roots[0]["updated"], json!(20.25));
        // 全整数的分支仍然输出整数，不能变成 3.0。
        let integers = vec![row(json!({"id": "x", "updated": 1, "count": 2, "size": 3}))];
        let roots = session_roots(integers).unwrap();
        assert_eq!(roots[0]["count"], json!(2));
        assert!(roots[0]["count"].is_i64());
    }

    #[test]
    fn missing_id_fails_instead_of_collapsing_rows() {
        // Python 是 `nodes[node["id"]]` → KeyError；塌成空串会把多条无 id 行合并。
        let error = session_roots(vec![row(json!({"updated": 1}))]).unwrap_err();
        assert_eq!(error.code, "internal.unexpected");
        assert_eq!(error.message(), "扫描行缺少 id");
    }

    #[test]
    fn non_string_ids_stay_distinct() {
        // Python 的 dict 键可以是 None / 数字：grok 的扫描行就允许 id 为 null。
        // 注意 Python 里缺 parent_id 的行会解析出 None 键，恰好命中 id 为 None
        // 的节点：所以这三行装配成「null 根 + 两个子节点」，而 7 与 "7" 是两个
        // 互不覆盖的键（Python 实测输出即此形状）。
        let rows = vec![
            row(json!({"id": null, "updated": 1})),
            row(json!({"id": 7, "updated": 2})),
            row(json!({"id": "7", "updated": 3})),
        ];
        let roots = session_roots(rows).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["id"], Value::Null);
        assert_eq!(roots[0]["child_count"], json!(2));
        assert_eq!(roots[0]["tree_count"], json!(3));
        let child_ids: Vec<&Value> = roots[0]["children"]
            .as_array()
            .unwrap()
            .iter()
            .map(|child| &child["id"])
            .collect();
        assert!(child_ids.contains(&&json!(7)));
        assert!(child_ids.contains(&&json!("7")));
    }

    #[test]
    fn explicit_null_counter_is_not_silently_zero() {
        // `source.get("own_count", ...)` 只在**缺键**时回落；显式 None 在 Python
        // 里会在后续 `+` 上抛 TypeError。
        let error = session_roots(vec![row(json!({"id": "a", "own_count": null}))]).unwrap_err();
        assert_eq!(error.code, "internal.unexpected");
        assert!(error.message().contains("own_count"));
    }
}
