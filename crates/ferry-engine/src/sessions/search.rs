//! 跨 Agent 会话索引搜索。
//!
//! 覆盖度必须如实上报：`partially_indexed_messages`（16 KB 盲区）、
//! `clipped_sessions_not_scanned`（预过滤跳过的截断会话）、`regex_scan.*`
//! （扫描/跳过/失败与 skip_reason）、`content_index.*`（索引就绪度）。
//! 模型只有拿到这些数字，才知道「没搜到」是不是等于「不存在」。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use regex::Regex;
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};

use super::agent_read::read_indexed_session;
use super::content_index::{
    has_boolean_operator, parse_query_terms, ContentHit, ContentIndex, SessionKey,
};
use super::index::{stable_json, AgentSessionIndex, IndexedSession};
use super::regex_search;
use super::safety::{
    bounded_int, finalize_dto, now_ms, python_json_len, record_session_id, string_set,
    truncate_text, validated_interval, MAX_AGENT_DTO_BYTES,
};
use super::usage::casefold;

pub const MAX_SEARCH_RESULTS: i64 = 50;
pub const MAX_OR_PATTERNS: usize = 16;
const SCOPES: [&str; 3] = ["any", "metadata", "content"];
const SNIPPET_CHARS: usize = 320;
/// 正则扫描读的是原始转录，必须有界：runtime 侧 25s 超时，留出余量。
const SCAN_TIME_BUDGET: Duration = Duration::from_millis(15_000);
const SCAN_BYTE_BUDGET: i64 = 256 * 1024 * 1024;

pub const UI_SEARCH_LIMIT: i64 = 30;
/// ⌘K 结果行只有一两行高，再长的片段渲染时也会被截掉。
const UI_SNIPPET_CHARS: usize = 160;

fn request_error(message: impl Into<String>, params: Map<String, Value>) -> DomainError {
    DomainError::new(
        "agent.request_invalid",
        "AgentRequestError",
        message,
        params,
    )
}

fn field(name: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("field".into(), Value::from(name));
    params
}

/// 分发层传来的 scope 原值：缺键才是 `"any"`，其余（含显式 `null` 与非字符串）
/// 一律走白名单校验并报错（`search.py:46-51` 的 `scope not in _SCOPES`）。
fn scope_text(raw: Option<&Value>) -> &str {
    match raw {
        None => "any",
        // 非字符串在 Python 里也进不了 `_SCOPES`，交给白名单统一报错。
        Some(value) => value.as_str().unwrap_or(""),
    }
}

fn validated_scope(scope: &str, needles: &[String], has_regex: bool) -> DomainResult<String> {
    if !SCOPES.contains(&scope) {
        let mut params = field("scope");
        params.insert(
            "accepts".into(),
            Value::Array(SCOPES.iter().map(|item| Value::from(*item)).collect()),
        );
        return Err(request_error("scope 仅允许 any/metadata/content", params));
    }
    if scope == "content" && needles.is_empty() && !has_regex {
        return Err(request_error(
            "scope=content 需要非空 query、patterns 或 regex",
            field("query"),
        ));
    }
    Ok(scope.to_string())
}

/// 把 query 与 patterns 归一成一组 OR pattern（原串，未折叠）。
///
/// 任一 pattern 命中即算命中，单个 pattern 内部仍是词级 AND。
fn validated_patterns(query: &str, patterns: Option<&Value>) -> DomainResult<Vec<String>> {
    let mut needles = Vec::new();
    validate_boolean_operators(query)?;
    if !query.trim().is_empty() {
        needles.push(query.trim().to_string());
    }
    let Some(patterns) = patterns.filter(|value| !value.is_null()) else {
        return Ok(needles);
    };
    let Some(items) = patterns
        .as_array()
        .filter(|items| items.len() <= MAX_OR_PATTERNS)
    else {
        return Err(request_error(
            format!("patterns 必须是至多 {MAX_OR_PATTERNS} 项的字符串数组"),
            field("patterns"),
        ));
    };
    for pattern in items {
        let text = pattern
            .as_str()
            .filter(|value| value.chars().count() <= 500)
            .ok_or_else(|| {
                request_error(
                    "patterns 每项必须是不超过 500 字符的字符串",
                    field("patterns"),
                )
            })?;
        validate_boolean_operators(text)?;
        if !text.trim().is_empty() {
            needles.push(text.trim().to_string());
        }
    }
    Ok(needles)
}

fn validate_boolean_operators(query: &str) -> DomainResult<()> {
    if has_boolean_operator(query) {
        let mut params = field("query");
        params.insert("reason".into(), Value::from("unsupported_boolean_operator"));
        params.insert(
            "recovery".into(),
            Value::from("OR 检索请使用多个 --pattern；字面 AND/OR/NOT 请加双引号"),
        );
        return Err(request_error("不支持裸 AND/OR/NOT 布尔操作符", params));
    }
    Ok(())
}

fn fingerprint(value: &Value) -> String {
    crate::jsonutil::sha256_hex(stable_json(value).as_bytes())
}

fn cursor_error(reason: &str) -> DomainError {
    let mut params = field("cursor");
    params.insert("reason".into(), Value::from(reason));
    params.insert(
        "recovery".into(),
        Value::from("移除 cursor，从第一页重新检索；续页必须使用相同查询和绝对时间范围"),
    );
    request_error("搜索游标无效、查询不一致或结果快照已变化", params)
}

fn encode_cursor(query: &str, snapshot: &str, offset: usize) -> Value {
    Value::from(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({"v": 1, "query": query, "snapshot": snapshot, "offset": offset})).expect("cursor serializes")
    ))
}

fn cursor_offset(
    raw: Option<&Value>,
    query: &str,
    snapshot: &str,
    total: usize,
) -> DomainResult<usize> {
    let Some(raw) = raw else {
        return Ok(0);
    };
    let text = raw
        .as_str()
        .filter(|text| text.len() <= 2048)
        .ok_or_else(|| cursor_error("cursor_invalid"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| cursor_error("cursor_invalid"))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| cursor_error("cursor_invalid"))?;
    if value["v"] != 1 {
        return Err(cursor_error("cursor_invalid"));
    }
    if value["query"] != query {
        return Err(cursor_error("cursor_mismatch"));
    }
    if value["snapshot"] != snapshot {
        return Err(cursor_error("cursor_stale"));
    }
    let offset = value["offset"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|offset| *offset > 0 && *offset < total)
        .ok_or_else(|| cursor_error("cursor_invalid"))?;
    Ok(offset)
}

/// `isinstance(value, bool)` 的等价校验。
///
/// **显式 `null` 也要报错**：分发层是 `p.get(key, False)`，只有**缺键**才落默认
/// 值，键在而值为 `null` 会原样传到这里，Python 的 `isinstance(None, bool)` 为
/// 假（`search.py:157-162`、`agent_read.py:400-401`）。
fn as_bool(value: Option<&Value>, message: &str, params: Map<String, Value>) -> DomainResult<bool> {
    match value {
        None => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(request_error(message, params)),
    }
}

/// 元数据过滤后的候选行。
struct Candidate {
    record: IndexedSession,
    project: String,
    project_truncated: bool,
    updated: i64,
    haystack: String,
    raw_haystack: String,
}

impl Candidate {
    fn key(&self) -> SessionKey {
        (self.record.tool.clone(), self.record.canonical_ref.clone())
    }
}

/// 对候选会话的原始转录跑正则，返回 (命中, 覆盖度元信息)。
#[allow(clippy::too_many_arguments)]
fn scan_regex(
    filtered: &[Candidate],
    compiled: &Regex,
    include_tool_outputs: bool,
    index: &Arc<AgentSessionIndex>,
    candidates: Option<&HashSet<SessionKey>>,
    clipped_by_session: &HashMap<SessionKey, i64>,
) -> (HashMap<SessionKey, ContentHit>, Map<String, Value>) {
    let mut ordered: Vec<&Candidate> = filtered.iter().collect();
    ordered.sort_by(|left, right| {
        right
            .updated
            .cmp(&left.updated)
            .then(left.key().cmp(&right.key()))
    });
    let deadline = Instant::now() + SCAN_TIME_BUDGET;
    let (mut scanned, mut skipped, mut read_failures, mut bytes_read) = (0i64, 0i64, 0i64, 0i64);
    let mut skip_reason: Option<&'static str> = None;
    let mut clipped_not_scanned = 0i64;
    let mut hits: HashMap<SessionKey, ContentHit> = HashMap::new();

    for candidate in ordered {
        let key = candidate.key();
        if let Some(allowed) = candidates {
            if !allowed.contains(&key) {
                // 字面量可能恰好只出现在没进索引的尾部：如实计数，
                // 模型看到这个数字非零且结果存疑时应改用 exhaustive 重扫。
                if clipped_by_session.contains_key(&key) {
                    clipped_not_scanned += 1;
                }
                continue;
            }
        }
        if skip_reason.is_some() {
            skipped += 1;
            continue;
        }
        if Instant::now() > deadline {
            skip_reason = Some("time_budget");
            skipped += 1;
            continue;
        }
        let size = candidate
            .record
            .row
            .get("size")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if scanned > 0 && bytes_read + size > SCAN_BYTE_BUDGET {
            skip_reason = Some("byte_budget");
            skipped += 1;
            continue;
        }
        // 会话正被写入等瞬态失败：跳过但计数，覆盖度必须如实。
        let Ok(session) = read_indexed_session(index, &candidate.record, true) else {
            read_failures += 1;
            continue;
        };
        scanned += 1;
        bytes_read += size;
        let (count, rows) = regex_search::scan_session(&session, compiled, include_tool_outputs);
        if count > 0 {
            hits.insert(
                key,
                ContentHit {
                    count,
                    best_rank: None,
                    rows,
                },
            );
        }
    }

    let mut meta = Map::new();
    meta.insert(
        "mode".into(),
        Value::from(if candidates.is_some() {
            "prefilter"
        } else {
            "full"
        }),
    );
    meta.insert("scanned_sessions".into(), Value::from(scanned));
    meta.insert("skipped_sessions".into(), Value::from(skipped));
    meta.insert(
        "skip_reason".into(),
        skip_reason.map(Value::from).unwrap_or(Value::Null),
    );
    meta.insert("read_failures".into(), Value::from(read_failures));
    if candidates.is_some() {
        meta.insert(
            "clipped_sessions_not_scanned".into(),
            Value::from(clipped_not_scanned),
        );
    }
    (hits, meta)
}

/// `content_search` 的入参；字段与 RPC 参数同名。
#[derive(Default)]
pub struct SearchRequest<'a> {
    pub query: Option<&'a Value>,
    pub agents: Option<&'a Value>,
    pub projects: Option<&'a Value>,
    /// 原生会话 id 精确过滤（大小写不敏感）；可与 agents/projects 组合，可无 query。
    pub session_ids: Option<&'a Value>,
    pub time_range: Option<&'a Value>,
    pub limit: Option<&'a Value>,
    pub cursor: Option<&'a Value>,
    pub scope: Option<&'a Value>,
    pub include_tool_outputs: Option<&'a Value>,
    pub patterns: Option<&'a Value>,
    pub regex: Option<&'a Value>,
    pub exhaustive: Option<&'a Value>,
}

pub fn search_sessions(
    request: &SearchRequest<'_>,
    index: &Arc<AgentSessionIndex>,
    content_index: Option<&Arc<ContentIndex>>,
) -> DomainResult<Map<String, Value>> {
    let limit = bounded_int(request.limit, 20, 1, MAX_SEARCH_RESULTS, "limit")? as usize;
    // 与 `as_bool` 同理：`p.get("query", "")` 只在缺键时给默认值，显式 null 会
    // 走到 `isinstance(None, str)` 的假分支（`search.py:154-155`）。
    let query = match request.query {
        None => "",
        Some(Value::String(text)) if text.chars().count() <= 500 => text.as_str(),
        Some(_) => {
            return Err(request_error(
                "query 必须是不超过 500 字符的字符串",
                Map::new(),
            ))
        }
    };
    let include_tool_outputs = as_bool(
        request.include_tool_outputs,
        "include_tool_outputs 必须是 boolean",
        Map::new(),
    )?;
    let exhaustive = as_bool(
        request.exhaustive,
        "exhaustive 必须是 boolean",
        field("exhaustive"),
    )?;
    let raw_regex = request.regex.filter(|value| !value.is_null());
    let compiled = match raw_regex {
        Some(pattern) => Some(regex_search::compile_regex(Some(pattern))?),
        None => None,
    };
    if exhaustive && compiled.is_none() {
        return Err(request_error(
            "exhaustive 仅与 regex 搭配使用",
            field("exhaustive"),
        ));
    }
    let mut allowed_agents = string_set(request.agents, "agents", 8, 32)?;
    allowed_agents.sort_unstable();
    // Python 侧是 `{item.casefold() for item in ...}`：折叠后还要再去一次重。
    let mut allowed_projects: Vec<String> = string_set(request.projects, "projects", 20, 256)?
        .iter()
        .map(|item| casefold(item))
        .collect();
    allowed_projects.sort_unstable();
    allowed_projects.dedup();
    let mut allowed_session_ids: Vec<String> =
        string_set(request.session_ids, "session_ids", 20, 256)?
            .iter()
            .map(|item| casefold(item))
            .collect();
    allowed_session_ids.sort_unstable();
    allowed_session_ids.dedup();
    let (start, end) = validated_interval(request.time_range)?;
    let needles = validated_patterns(query, request.patterns)?;
    if compiled.is_some() && !needles.is_empty() {
        return Err(request_error(
            "regex 不能与 query/patterns 同用",
            field("regex"),
        ));
    }
    let has_needle = !needles.is_empty() || compiled.is_some();
    let scope = validated_scope(scope_text(request.scope), &needles, compiled.is_some())?;
    // 每个 pattern 拆词后折叠；OR 关系：任一 pattern 的词全部命中即算。
    let needle_groups: Vec<Vec<String>> = needles
        .iter()
        .map(|needle| {
            parse_query_terms(needle)
                .iter()
                .map(|term| casefold(term))
                .collect()
        })
        .collect();

    let query_signature = fingerprint(&serde_json::json!({
        "needles": needles, "regex": raw_regex, "scope": scope,
        "agents": allowed_agents, "projects": allowed_projects, "session_ids": allowed_session_ids,
        "time_range": [start, end], "include_tool_outputs": include_tool_outputs, "exhaustive": exhaustive
    }));
    let records = index.refresh_selected(&allowed_agents)?;
    // 元数据过滤前置：正则扫描只扫过滤后的会话，主循环复用同一份。
    let mut filtered: Vec<Candidate> = Vec::new();
    for record in &records {
        let row = &record.row;
        let (project, project_truncated) = truncate_text(
            row.get("dir").and_then(Value::as_str).unwrap_or_default(),
            1024,
        );
        let updated = row.get("updated").and_then(Value::as_i64).unwrap_or(0);
        let raw_haystack = format!(
            "{} {} {} {}",
            row.get("title").and_then(Value::as_str).unwrap_or_default(),
            project,
            record.tool,
            row.get("model").and_then(Value::as_str).unwrap_or_default(),
        );
        if !allowed_agents.is_empty() && !allowed_agents.contains(&record.tool) {
            continue;
        }
        if !allowed_projects.is_empty()
            && !allowed_projects.contains(&casefold(
                row.get("dir").and_then(Value::as_str).unwrap_or_default(),
            ))
        {
            continue;
        }
        if !allowed_session_ids.is_empty()
            && !allowed_session_ids.contains(&casefold(&record_session_id(row, None)))
        {
            continue;
        }
        if start.is_some_and(|start| updated < start) || end.is_some_and(|end| updated > end) {
            continue;
        }
        filtered.push(Candidate {
            record: record.clone(),
            project,
            project_truncated,
            updated,
            haystack: casefold(&raw_haystack),
            raw_haystack,
        });
    }

    // 内容命中与索引覆盖度。索引不可用时特性显式降级：词法搜索退成纯元数据
    // 搜索，正则搜索退成全量扫描（它本就不依赖索引的正确性）。
    let mut content_hits: HashMap<SessionKey, ContentHit> = HashMap::new();
    let mut content_status: Option<Map<String, Value>> = None;
    let mut clipped_by_session: HashMap<SessionKey, i64> = HashMap::new();
    let content_active = has_needle && scope != "metadata";
    if content_active {
        let mut status = match content_index {
            None => {
                let mut status = Map::new();
                status.insert("ready".into(), Value::Bool(false));
                status.insert("reason".into(), Value::from("content_index_unavailable"));
                status
            }
            Some(content) => {
                content.sync(index, &records, false)?;
                let scoped_records: Vec<IndexedSession> = filtered
                    .iter()
                    .map(|candidate| candidate.record.clone())
                    .collect();
                let status = content.candidate_coverage(&scoped_records)?;
                clipped_by_session = content.clipped_rows_by_session()?;
                status
            }
        };
        if let Some(compiled) = compiled.as_ref() {
            let mut candidates: Option<HashSet<SessionKey>> = None;
            let literals = if exhaustive {
                Vec::new()
            } else {
                regex_search::required_literals(
                    raw_regex.and_then(Value::as_str).unwrap_or_default(),
                )
            };
            // 旧 revision 的索引不能证明当前原文不存在字面词；未同步时退回有界原文扫描。
            if !literals.is_empty() && status.get("ready") == Some(&Value::Bool(true)) {
                if let Some(content) = content_index {
                    // 预过滤 = 字面量命中 ∪ 尚未入索引的会话：索引「说没有」
                    // 才可跳过，「还不知道」（构建中/刚写入）必须进扫描候选。
                    let matched =
                        content.sessions_matching_literals(&literals, include_tool_outputs)?;
                    let known = content.indexed_session_keys()?;
                    if let (Some(mut matched), Some(known)) = (matched, known) {
                        for candidate in &filtered {
                            let key = candidate.key();
                            if !known.contains(&key) {
                                matched.insert(key);
                            }
                        }
                        candidates = Some(matched);
                    }
                }
            }
            let (hits, scan_meta) = scan_regex(
                &filtered,
                compiled,
                include_tool_outputs,
                index,
                candidates.as_ref(),
                &clipped_by_session,
            );
            content_hits = hits;
            status.insert("match_mode".into(), Value::from("regex"));
            status.insert("regex_scan".into(), Value::Object(scan_meta));
        } else if let Some(content) = content_index {
            let candidate_keys: Vec<SessionKey> = filtered.iter().map(Candidate::key).collect();
            let (hits, meta) =
                content.search_candidates(&needles, include_tool_outputs, Some(&candidate_keys))?;
            content_hits = hits;
            for (key, value) in meta {
                status.insert(key, value);
            }
        }
        content_status = Some(status);
    }

    struct Scored {
        item: Map<String, Value>,
        hit: Option<ContentHit>,
        rank: (i64, f64, i64),
        updated: i64,
        key: SessionKey,
    }

    let mut matches: Vec<Scored> = Vec::new();
    for candidate in &filtered {
        let record = &candidate.record;
        let row = &record.row;
        let metadata_hit = match compiled.as_ref() {
            Some(compiled) => compiled.is_match(&candidate.raw_haystack),
            None => needle_groups.iter().any(|group| {
                group
                    .iter()
                    .all(|term| candidate.haystack.contains(term.as_str()))
            }),
        };
        let content_hit = content_hits.get(&candidate.key()).cloned();
        if has_needle {
            let keep = match scope.as_str() {
                "metadata" => metadata_hit,
                "content" => content_hit.is_some(),
                _ => metadata_hit || content_hit.is_some(),
            };
            if !keep {
                continue;
            }
        }
        let (title, title_truncated) = truncate_text(
            row.get("title").and_then(Value::as_str).unwrap_or_default(),
            200,
        );
        let (model, model_truncated) = truncate_text(
            row.get("model").and_then(Value::as_str).unwrap_or_default(),
            120,
        );
        let mut item = Map::new();
        item.insert("tool".into(), Value::from(record.tool.as_str()));
        item.insert("ref".into(), Value::from(record.opaque_ref.as_str()));
        item.insert(
            "session_id".into(),
            Value::from(record_session_id(row, None)),
        );
        item.insert("title".into(), Value::from(title));
        item.insert(
            "title_source".into(),
            Value::from(
                row.get("title_source")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            ),
        );
        item.insert("project".into(), Value::from(candidate.project.as_str()));
        item.insert("title_truncated".into(), Value::Bool(title_truncated));
        item.insert(
            "project_truncated".into(),
            Value::Bool(candidate.project_truncated),
        );
        item.insert("updated".into(), Value::from(candidate.updated));
        // 与 session_read 的 message_count 不是一回事：这里是原始转录条数。
        item.insert(
            "record_count".into(),
            Value::from(row.get("count").and_then(Value::as_i64).unwrap_or(0)),
        );
        item.insert("model".into(), Value::from(model));
        item.insert("model_truncated".into(), Value::Bool(model_truncated));
        item.insert("revision".into(), Value::from(record.revision.as_str()));
        if has_needle && content_active {
            let mut matched_in = Vec::new();
            if metadata_hit {
                matched_in.push(Value::from("metadata"));
            }
            if let Some(hit) = content_hit.as_ref() {
                matched_in.push(Value::from("content"));
                item.insert("content_match_count".into(), Value::from(hit.count));
            }
            item.insert("matched_in".into(), Value::Array(matched_in));
        }
        if content_active {
            // 盲区透出：这些消息只有前 16 KB 进了索引，词法搜索「没命中」
            // 不等于「不存在」；模型可据此升级到 regex 原文扫描。
            if let Some(clipped) = clipped_by_session.get(&candidate.key()) {
                if *clipped > 0 {
                    item.insert("partially_indexed_messages".into(), Value::from(*clipped));
                }
            }
        }
        // 有内容命中时按相关性分组排序：双命中 > 内容命中(bm25) > 纯元数据。
        let group = match content_hit.as_ref() {
            Some(_) if metadata_hit => 0,
            Some(_) => 1,
            None => 2,
        };
        let rank = content_hit
            .as_ref()
            .and_then(|hit| hit.best_rank)
            .unwrap_or(f64::INFINITY);
        matches.push(Scored {
            item,
            hit: content_hit,
            rank: (group, rank, -candidate.updated),
            updated: candidate.updated,
            key: candidate.key(),
        });
    }

    if content_active && !content_hits.is_empty() {
        matches.sort_by(|left, right| {
            left.rank
                .0
                .cmp(&right.rank.0)
                .then(
                    left.rank
                        .1
                        .partial_cmp(&right.rank.1)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(left.rank.2.cmp(&right.rank.2))
                .then(left.key.cmp(&right.key))
        });
    } else {
        matches.sort_by(|left, right| {
            right
                .updated
                .cmp(&left.updated)
                .then(left.key.cmp(&right.key))
        });
    }

    let mut reasons: Vec<&str> = Vec::new();
    if index.snapshot_with_status().is_some_and(|(tools, _, _)| {
        tools.iter().any(|(name, status)| {
            (allowed_agents.is_empty() || allowed_agents.contains(name)) && status["ok"] == false
        })
    }) {
        reasons.push("session_scan_failed");
    }
    if let Some(status) = content_status.as_ref() {
        if let Some(scan) = status.get("regex_scan") {
            if scan["skipped_sessions"].as_i64().unwrap_or(0) > 0 {
                reasons.push("regex_scan_budget");
            }
            if scan["read_failures"].as_i64().unwrap_or(0) > 0 {
                reasons.push("regex_read_failures");
            }
            if scan["clipped_sessions_not_scanned"].as_i64().unwrap_or(0) > 0 {
                reasons.push("clipped_sessions_not_scanned");
            }
        } else {
            if status.get("ready") != Some(&Value::Bool(true)) {
                reasons.push("content_index_not_ready");
            }
            if status
                .get("failed_sessions")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                > 0
            {
                reasons.push("content_index_read_failures");
            }
            if status
                .get("partially_indexed_messages")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                > 0
            {
                reasons.push("partially_indexed_messages");
            }
            if status.get("rows_capped") == Some(&Value::Bool(true)) {
                reasons.push("content_match_rows_capped");
            }
        }
    }
    let complete = reasons.is_empty();
    // 游标钉住候选 revision、完整排序结果和覆盖度；索引收敛或源变化时显式拒绝旧页。
    let mut revisions: Vec<Value> = filtered
        .iter()
        .map(|candidate| {
            serde_json::json!([
                candidate.record.tool,
                candidate.record.canonical_ref,
                candidate.record.revision
            ])
        })
        .collect();
    revisions.sort_by_key(stable_json);
    let snapshot = fingerprint(&serde_json::json!({
        "revisions": revisions,
        "results": matches.iter().map(|scored| serde_json::json!({
            "item": scored.item,
            "hit": scored.hit.as_ref().map(|hit| serde_json::json!({"count": hit.count, "rank": hit.best_rank, "rows": hit.rows}))
        })).collect::<Vec<_>>(),
        "coverage": reasons
    }));
    let total_matches = matches.len();
    let offset = cursor_offset(request.cursor, &query_signature, &snapshot, total_matches)?;

    // 摘要只为最终返回页生成，避免为未返回结果做额外读取。
    // regex 扫描的命中行自带原文摘要；词法命中回索引取上下文窗口。
    if content_active {
        for scored in matches.iter_mut().skip(offset).take(limit) {
            let Some(hit) = scored.hit.as_ref() else {
                continue;
            };
            let content_matches: Vec<Value> = hit
                .rows
                .iter()
                .map(|row| -> DomainResult<Value> {
                    let snippet = match row.get("snippet").and_then(Value::as_str) {
                        Some(text) => Ok(text.to_string()),
                        None => match (content_index, row.get("id").and_then(Value::as_i64)) {
                            (Some(content), Some(id)) => {
                                content.snippet(id, &needles, include_tool_outputs)
                            }
                            _ => Ok(String::new()),
                        },
                    }?;
                    let mut entry = Map::new();
                    for key in ["message", "turn", "role"] {
                        entry.insert(key.into(), row.get(key).cloned().unwrap_or(Value::Null));
                    }
                    entry.insert(
                        "snippet".into(),
                        Value::from(truncate_text(&snippet, SNIPPET_CHARS).0),
                    );
                    Ok(Value::Object(entry))
                })
                .collect::<DomainResult<Vec<Value>>>()?;
            scored
                .item
                .insert("content_matches".into(), Value::Array(content_matches));
        }
    }

    let mut result = serde_json::json!({
        "sessions": [], "returned": 0, "total_matches": total_matches,
        "total_matches_relation": if complete { "eq" } else { "gte" },
        "has_more": false, "next_cursor": null,
        "coverage": { "complete": complete, "reasons": reasons },
        "resolved_time_range": { "from": start, "to": end },
        "now": now_ms(),
        "truncation": { "truncated": false, "reason": null, "budget_bytes": MAX_AGENT_DTO_BYTES }
    })
    .as_object()
    .unwrap()
    .clone();
    if let Some(status) = content_status {
        result.insert("content_index".into(), Value::Object(status));
    }
    let mut selected = Vec::new();
    let mut byte_limited = false;
    for scored in matches.iter().skip(offset).take(limit) {
        selected.push(Value::Object(scored.item.clone()));
        let mut probe = result.clone();
        update_page(
            &mut probe,
            &selected,
            offset,
            total_matches,
            &query_signature,
            &snapshot,
        );
        // 截断标记也算入预算，不依赖固定预留字节。
        probe.insert("truncation".into(), serde_json::json!({"truncated": true, "reason": "byte_budget", "budget_bytes": MAX_AGENT_DTO_BYTES}));
        if python_json_len(&Value::Object(probe)) > MAX_AGENT_DTO_BYTES {
            selected.pop();
            byte_limited = true;
            break;
        }
    }
    if byte_limited && selected.is_empty() {
        return Err(request_error("单个搜索结果超过响应预算", field("limit")));
    }
    update_page(
        &mut result,
        &selected,
        offset,
        total_matches,
        &query_signature,
        &snapshot,
    );
    if byte_limited {
        result.insert("truncation".into(), serde_json::json!({"truncated": true, "reason": "byte_budget", "budget_bytes": MAX_AGENT_DTO_BYTES}));
    }
    finalize_dto(result)
}

fn update_page(
    result: &mut Map<String, Value>,
    selected: &[Value],
    offset: usize,
    total: usize,
    query: &str,
    snapshot: &str,
) {
    let next = offset + selected.len();
    result.insert("sessions".into(), Value::Array(selected.to_vec()));
    result.insert("returned".into(), Value::from(selected.len()));
    result.insert("has_more".into(), Value::Bool(next < total));
    result.insert(
        "next_cursor".into(),
        if next < total {
            encode_cursor(query, snapshot, next)
        } else {
            Value::Null
        },
    );
}

/// UI 用 title 称呼元数据档位；两个词都收，避免前端再维护一张映射表。
fn ui_scope(scope: &str) -> Option<&'static str> {
    match scope {
        "any" => Some("any"),
        "title" | "metadata" => Some("metadata"),
        "content" => Some("content"),
        _ => None,
    }
}

/// UI 的 limit 钳制而不是报错：搜索框不该因为翻页参数越界整个空掉。
fn ui_clamped_limit(value: Option<&Value>) -> DomainResult<i64> {
    match value {
        None | Some(Value::Null) => Ok(UI_SEARCH_LIMIT),
        Some(Value::Number(number)) if number.is_i64() || number.is_u64() => {
            let raw = number.as_i64().unwrap_or(UI_SEARCH_LIMIT);
            Ok(raw.clamp(1, MAX_SEARCH_RESULTS))
        }
        Some(_) => Err(request_error("limit 必须是整数", field("limit"))),
    }
}

/// 全局搜索的 UI 视图：每个会话只留最佳片段，字段收窄到结果行要用的。
pub fn search_sessions_for_ui(
    query: Option<&Value>,
    tools: Option<&Value>,
    limit: Option<&Value>,
    scope: Option<&Value>,
    index: &Arc<AgentSessionIndex>,
    content_index: Option<&Arc<ContentIndex>>,
) -> DomainResult<Map<String, Value>> {
    let text = query
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| request_error("query 必须是非空字符串", field("query")))?;
    let requested_scope = scope_text(scope);
    let Some(mapped) = ui_scope(requested_scope) else {
        let mut params = field("scope");
        params.insert(
            "accepts".into(),
            Value::Array(
                ["any", "title", "content"]
                    .iter()
                    .map(|item| Value::from(*item))
                    .collect(),
            ),
        );
        return Err(request_error("scope 仅允许 any/title/content", params));
    };
    let clamped = Value::from(ui_clamped_limit(limit)?);
    let mapped_scope = Value::from(mapped);
    let query_value = Value::from(text);
    let raw = search_sessions(
        &SearchRequest {
            query: Some(&query_value),
            agents: tools,
            limit: Some(&clamped),
            scope: Some(&mapped_scope),
            ..SearchRequest::default()
        },
        index,
        content_index,
    )?;

    let mut sessions: Vec<Value> = Vec::new();
    for item in raw
        .get("sessions")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let best = item
            .get("content_matches")
            .and_then(Value::as_array)
            .and_then(|rows| rows.first());
        let mut entry = Map::new();
        for key in ["tool", "ref", "title", "title_source", "project", "updated"] {
            entry.insert(key.into(), item.get(key).cloned().unwrap_or(Value::Null));
        }
        // 无内容检索档位时 search_sessions 不写 matched_in，但结果本身就是
        // 元数据命中，这里补齐让前端只有一种分支。
        entry.insert(
            "matched_in".into(),
            match item.get("matched_in") {
                Some(Value::Array(items)) if !items.is_empty() => Value::Array(items.clone()),
                _ => Value::Array(vec![Value::from("metadata")]),
            },
        );
        entry.insert(
            "match_count".into(),
            item.get("content_match_count")
                .cloned()
                .unwrap_or(Value::from(0)),
        );
        entry.insert(
            "snippet".into(),
            Value::from(match best {
                Some(best) => {
                    truncate_text(
                        best.get("snippet").and_then(Value::as_str).unwrap_or(""),
                        UI_SNIPPET_CHARS,
                    )
                    .0
                }
                None => String::new(),
            }),
        );
        entry.insert(
            "message".into(),
            best.and_then(|best| best.get("message").cloned())
                .unwrap_or(Value::Null),
        );
        entry.insert(
            "role".into(),
            best.and_then(|best| best.get("role").cloned())
                .unwrap_or(Value::Null),
        );
        sessions.push(Value::Object(entry));
    }

    let mut result = Map::new();
    // 回显 query 让前端能丢弃被后续按键取代的过期响应。
    result.insert("query".into(), Value::from(text.trim()));
    result.insert("scope".into(), Value::from(requested_scope));
    let returned = sessions.len();
    result.insert("sessions".into(), Value::Array(sessions));
    result.insert("returned".into(), Value::from(returned));
    result.insert(
        "total_matches".into(),
        raw.get("total_matches").cloned().unwrap_or(Value::from(0)),
    );
    result.insert(
        "has_more".into(),
        raw.get("has_more").cloned().unwrap_or(Value::Bool(false)),
    );
    for key in ["content_index", "coverage", "total_matches_relation"] {
        if let Some(value) = raw.get(key) {
            result.insert(key.into(), value.clone());
        }
    }
    finalize_dto(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scope_validation_matches_python_wording() {
        assert_eq!(validated_scope("any", &[], false).unwrap(), "any");
        let error = validated_scope("nope", &[], false).unwrap_err();
        assert_eq!(error.message(), "scope 仅允许 any/metadata/content");
        assert_eq!(
            error.params()["accepts"],
            json!(["any", "metadata", "content"])
        );
        let empty = validated_scope("content", &[], false).unwrap_err();
        assert_eq!(
            empty.message(),
            "scope=content 需要非空 query、patterns 或 regex"
        );
        assert!(validated_scope("content", &[], true).is_ok());
        assert!(validated_scope("content", &["x".into()], false).is_ok());
    }

    /// 分发层是 `p.get(key, default)`：**缺键**才落默认值，键在而值为 `null`
    /// 要原样传下来并被 `isinstance` 判死。
    #[test]
    fn explicit_null_is_rejected_not_defaulted() {
        // scope：缺键 → "any"；显式 null / 非字符串 → 白名单报错。
        assert_eq!(scope_text(None), "any");
        assert_eq!(scope_text(Some(&Value::Null)), "");
        assert_eq!(scope_text(Some(&json!(5))), "");
        assert_eq!(scope_text(Some(&json!("content"))), "content");
        let error = validated_scope(scope_text(Some(&Value::Null)), &[], false).unwrap_err();
        assert_eq!(error.message(), "scope 仅允许 any/metadata/content");

        // include_tool_outputs / exhaustive。
        assert!(!as_bool(None, "boom", Map::new()).unwrap());
        assert!(as_bool(Some(&Value::Bool(true)), "boom", Map::new()).unwrap());
        for bad in [Value::Null, json!(0), json!("true")] {
            let error = as_bool(
                Some(&bad),
                "include_tool_outputs 必须是 boolean",
                Map::new(),
            )
            .unwrap_err();
            assert_eq!(error.message(), "include_tool_outputs 必须是 boolean");
        }
    }

    #[test]
    fn patterns_are_trimmed_and_bounded() {
        assert_eq!(
            validated_patterns("  hello  ", None).unwrap(),
            vec!["hello".to_string()]
        );
        assert!(validated_patterns("   ", None).unwrap().is_empty());
        assert_eq!(
            validated_patterns("a", Some(&json!(["b", "  ", " c "]))).unwrap(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
        let too_many: Vec<Value> = (0..17).map(|_| Value::from("x")).collect();
        assert!(validated_patterns("", Some(&Value::Array(too_many))).is_err());
        assert!(validated_patterns("", Some(&json!([1]))).is_err());
        assert!(validated_patterns("", Some(&json!("nope"))).is_err());
    }

    #[test]
    fn ui_limit_is_clamped_not_rejected() {
        assert_eq!(ui_clamped_limit(None).unwrap(), UI_SEARCH_LIMIT);
        assert_eq!(ui_clamped_limit(Some(&json!(0))).unwrap(), 1);
        assert_eq!(
            ui_clamped_limit(Some(&json!(999))).unwrap(),
            MAX_SEARCH_RESULTS
        );
        assert_eq!(ui_clamped_limit(Some(&json!(7))).unwrap(), 7);
        assert!(ui_clamped_limit(Some(&json!("x"))).is_err());
    }

    #[test]
    fn ui_scope_accepts_both_title_and_metadata() {
        assert_eq!(ui_scope("any"), Some("any"));
        assert_eq!(ui_scope("title"), Some("metadata"));
        assert_eq!(ui_scope("metadata"), Some("metadata"));
        assert_eq!(ui_scope("content"), Some("content"));
        assert_eq!(ui_scope("nope"), None);
    }

    /// 端到端：用黄金扫描行驱动的索引跑一遍元数据档位的检索。
    ///
    /// 内容档位需要真实 canonical Session（适配器 WP-C1..C5 尚未就绪），
    /// 这里只钉住元数据匹配、排序与覆盖度字段的全套形状。
    /// `session_ids`：原生 id 精确过滤（大小写不敏感），可无 query，可叠加 agents。
    #[test]
    fn session_ids_filter_matches_the_native_id_exactly() {
        let harness = crate::sessions::index::golden_tests::harness();
        let index = &harness.index;
        let all = search_sessions(&SearchRequest::default(), index, None).expect("空检索");
        let sample = all["sessions"][0].clone();
        let native = sample["session_id"]
            .as_str()
            .expect("有原生 id")
            .to_string();
        let tool = sample["tool"].as_str().expect("有 tool").to_string();

        // 大小写不敏感的精确命中，且照常给出 ref。
        let ids = Value::Array(vec![Value::from(native.to_uppercase())]);
        let hit = search_sessions(
            &SearchRequest {
                session_ids: Some(&ids),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .expect("按 session_id 过滤");
        assert_eq!(hit["returned"], Value::from(1));
        assert_eq!(
            hit["sessions"][0]["session_id"],
            Value::from(native.as_str())
        );
        assert!(hit["sessions"][0]["ref"]
            .as_str()
            .unwrap()
            .starts_with("fsr_"));

        // 与 agents 组合：同一个 agent 仍命中。
        let agents = Value::Array(vec![Value::from(tool.as_str())]);
        let combined = search_sessions(
            &SearchRequest {
                session_ids: Some(&ids),
                agents: Some(&agents),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .expect("agents + session_ids");
        assert_eq!(combined["returned"], Value::from(1));

        // 未命中给 0 而不是报错。
        let missing = Value::Array(vec![Value::from("no-such-session-id")]);
        let empty = search_sessions(
            &SearchRequest {
                session_ids: Some(&missing),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .expect("未命中");
        assert_eq!(empty["returned"], Value::from(0));
        assert_eq!(empty["total_matches"], Value::from(0));

        // 超过 20 项或空串一律报参数错误。
        let too_many = Value::Array((0..21).map(|i| Value::from(format!("id{i}"))).collect());
        assert!(search_sessions(
            &SearchRequest {
                session_ids: Some(&too_many),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .is_err());
    }

    #[test]
    fn metadata_search_end_to_end_reports_the_full_dto_shape() {
        let harness = crate::sessions::index::golden_tests::harness();
        let index = &harness.index;

        // 空 query：无检索词 → 返回全部 13 条，按 updated 降序。
        let all = search_sessions(&SearchRequest::default(), index, None).expect("空检索");
        assert_eq!(all["total_matches"], Value::from(13));
        assert_eq!(all["returned"], Value::from(13));
        assert_eq!(all["has_more"], Value::Bool(false));
        assert!(all["now"].as_i64().unwrap() > 1_700_000_000_000);
        assert_eq!(all["truncation"]["truncated"], Value::Bool(false));
        assert_eq!(all["truncation"]["reason"], Value::Null);
        assert_eq!(
            all["truncation"]["budget_bytes"],
            Value::from(MAX_AGENT_DTO_BYTES)
        );
        // 无检索词时不带 content_index 状态。
        assert!(all.get("content_index").is_none());
        let updated: Vec<i64> = all["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["updated"].as_i64().unwrap())
            .collect();
        let mut sorted = updated.clone();
        sorted.sort_unstable_by(|left, right| right.cmp(left));
        assert_eq!(updated, sorted);
        // 结果行字段齐全。
        let first = &all["sessions"][0];
        for key in [
            "tool",
            "ref",
            "session_id",
            "title",
            "project",
            "title_truncated",
            "project_truncated",
            "updated",
            "record_count",
            "model",
            "model_truncated",
            "revision",
        ] {
            assert!(first.get(key).is_some(), "缺少字段 {key}");
        }
        assert!(first["ref"].as_str().unwrap().starts_with("fsr_"));

        // 元数据档位：按 tool 名过滤（haystack 含 tool）。
        let query = Value::from("grok");
        let scope = Value::from("metadata");
        let hit = search_sessions(
            &SearchRequest {
                query: Some(&query),
                scope: Some(&scope),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .expect("元数据检索");
        assert_eq!(hit["total_matches"], Value::from(4));
        // metadata 档位不做内容检索，也就没有 content_index 状态。
        assert!(hit.get("content_index").is_none());

        // any 档位 + 无内容索引：显式降级并如实上报。
        let degraded = search_sessions(
            &SearchRequest {
                query: Some(&query),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .expect("降级检索");
        assert_eq!(
            degraded["content_index"],
            serde_json::json!({"ready": false, "reason": "content_index_unavailable"})
        );
        assert_eq!(degraded["total_matches"], Value::from(4));
        assert_eq!(
            degraded["sessions"][0]["matched_in"],
            serde_json::json!(["metadata"])
        );

        // agents 过滤 + limit 截断。
        let agents = serde_json::json!(["claude"]);
        let limit = Value::from(1);
        let narrowed = search_sessions(
            &SearchRequest {
                agents: Some(&agents),
                limit: Some(&limit),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .expect("按 agent 过滤");
        assert_eq!(narrowed["total_matches"], Value::from(2));
        assert_eq!(narrowed["returned"], Value::from(1));
        assert_eq!(narrowed["has_more"], Value::Bool(true));

        // regex 与 query 互斥。
        let pattern = Value::from("fixture");
        let clash = search_sessions(
            &SearchRequest {
                query: Some(&query),
                regex: Some(&pattern),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .unwrap_err();
        assert_eq!(clash.message(), "regex 不能与 query/patterns 同用");

        // exhaustive 必须配 regex。
        let exhaustive = Value::Bool(true);
        let lonely = search_sessions(
            &SearchRequest {
                exhaustive: Some(&exhaustive),
                ..SearchRequest::default()
            },
            index,
            None,
        )
        .unwrap_err();
        assert_eq!(lonely.message(), "exhaustive 仅与 regex 搭配使用");
    }

    /// UI 视图：字段收窄、limit 钳制、query 回显。
    #[test]
    fn ui_search_narrows_the_result_rows() {
        let harness = crate::sessions::index::golden_tests::harness();
        let query = Value::from("  grok  ");
        let scope = Value::from("title");
        let result = search_sessions_for_ui(
            Some(&query),
            None,
            Some(&Value::from(2)),
            Some(&scope),
            &harness.index,
            None,
        )
        .expect("UI 检索");
        assert_eq!(result["query"], Value::from("grok"));
        assert_eq!(result["scope"], Value::from("title"));
        assert_eq!(result["returned"], Value::from(2));
        assert_eq!(result["total_matches"], Value::from(4));
        assert_eq!(result["has_more"], Value::Bool(true));
        let row = &result["sessions"][0];
        assert_eq!(row["matched_in"], serde_json::json!(["metadata"]));
        assert_eq!(row["match_count"], Value::from(0));
        assert_eq!(row["snippet"], Value::from(""));
        assert_eq!(row["message"], Value::Null);
        // 收窄后不再暴露 revision / record_count 之类的 agent 专用字段。
        assert!(row.get("revision").is_none());
    }
}

#[cfg(test)]
mod pagination_tests {
    use super::*;
    use crate::adapters::contracts::{
        id_reference, AgentAdapter, AgentManifest, Fingerprint, NativeSessionReference, ScanCache,
        ScanRow, SessionBrowser,
    };
    use crate::model::Session;
    use serde_json::json;
    use std::sync::Mutex;

    struct Browser {
        rows: Mutex<Vec<ScanRow>>,
    }
    impl SessionBrowser for Browser {
        fn scan(&self, _: &dyn ScanCache) -> DomainResult<Vec<ScanRow>> {
            Ok(self.rows.lock().unwrap().clone())
        }
        fn read(&self, reference: &str) -> DomainResult<Session> {
            let mut session = Session::new("opencode", reference, "/fixture");
            if let Some(body) = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(reference))
                .and_then(|row| row.get("body"))
                .and_then(Value::as_str)
            {
                let mut message = crate::model::Message::new("user");
                message.blocks.push(crate::model::Block::text(body));
                session.messages.push(message);
            }
            Ok(session)
        }
        fn resolve_ref(&self, reference: &str) -> DomainResult<String> {
            Ok(reference.into())
        }
        fn fingerprint(&self, _: &str) -> DomainResult<Fingerprint> {
            Ok(json!("fixture"))
        }
        fn agent_fingerprint(&self, reference: &str) -> DomainResult<Fingerprint> {
            self.fingerprint(reference)
        }
        fn canonicalize(&self, row: &ScanRow) -> Option<NativeSessionReference> {
            id_reference(row)
        }
        fn validate_read_scope(&self, _: &NativeSessionReference) -> DomainResult<()> {
            Ok(())
        }
    }
    struct Ports {
        adapter: AgentAdapter,
        cache: Arc<dyn ScanCache>,
    }
    impl super::super::index::SessionPorts for Ports {
        fn adapter(&self, _: &str) -> DomainResult<&AgentAdapter> {
            Ok(&self.adapter)
        }
        fn adapters(&self) -> Vec<String> {
            vec!["opencode".into()]
        }
        fn cache_factory(&self) -> Arc<dyn ScanCache> {
            self.cache.clone()
        }
    }
    fn fixture(
        count: usize,
        large: bool,
    ) -> (tempfile::TempDir, Arc<AgentSessionIndex>, Arc<Browser>) {
        let temp = tempfile::tempdir().unwrap();
        let browser = Arc::new(Browser { rows: Mutex::new((0..count).rev().map(|i| json!({
            "id": format!("session-{i:04}"), "title": if large { "标".repeat(200) } else { "fixture".into() },
            "dir": if large { "项".repeat(1024) } else { "/fixture".into() },
            "model": if large { "模".repeat(120) } else { "test".into() },
            "updated": 100, "size": 0, "count": 1
        }).as_object().unwrap().clone()).collect()) });
        let adapter = AgentAdapter::builder()
            .browser(browser.clone())
            .build(AgentManifest {
                id: "opencode".into(),
                display_name: "fixture".into(),
                icon: "fixture".into(),
                source_path: temp.path().to_string_lossy().into(),
                capabilities: vec!["browse".into()],
                edit_operations: vec![],
                executables: vec![],
                fallback_bin_dirs: vec![],
            })
            .unwrap();
        let cache = Arc::new(super::super::scan_cache::ScanCache::new(Some(
            temp.path().join("scan.json"),
        )));
        let index = Arc::new(AgentSessionIndex::new(Arc::new(Ports { adapter, cache })));
        (temp, index, browser)
    }

    #[test]
    fn same_timestamp_sessions_page_without_omissions_or_duplicates() {
        let (_temp, index, _) = fixture(151, false);
        let mut cursor = None;
        let mut ids = Vec::new();
        let mut pages = 0;
        loop {
            let result = search_sessions(
                &SearchRequest {
                    limit: Some(&json!(50)),
                    cursor: cursor.as_ref(),
                    ..Default::default()
                },
                &index,
                None,
            )
            .unwrap();
            assert_eq!(result["total_matches"], 151);
            assert_eq!(result["total_matches_relation"], "eq");
            assert_eq!(result["coverage"]["complete"], true);
            ids.extend(
                result["sessions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["session_id"].as_str().unwrap().to_string()),
            );
            pages += 1;
            if result["has_more"] == false {
                assert!(result["next_cursor"].is_null());
                break;
            }
            cursor = Some(result["next_cursor"].clone());
            assert!(pages < 5);
        }
        assert_eq!(pages, 4);
        assert_eq!(
            ids,
            (0..151)
                .map(|i| format!("session-{i:04}"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn byte_budget_includes_cursor_and_metadata_and_continues_at_first_unreturned_item() {
        let (_temp, index, _) = fixture(70, true);
        let mut cursor = None;
        let mut ids = HashSet::new();
        let mut budget_pages = 0;
        loop {
            let result = search_sessions(
                &SearchRequest {
                    limit: Some(&json!(50)),
                    cursor: cursor.as_ref(),
                    ..Default::default()
                },
                &index,
                None,
            )
            .unwrap();
            assert!(python_json_len(&Value::Object(result.clone())) <= MAX_AGENT_DTO_BYTES);
            assert!(result["returned"].as_u64().unwrap() > 0);
            for row in result["sessions"].as_array().unwrap() {
                assert!(ids.insert(row["session_id"].as_str().unwrap().to_string()));
            }
            if result["truncation"]["truncated"] == true {
                budget_pages += 1;
            }
            if result["has_more"] == false {
                break;
            }
            cursor = Some(result["next_cursor"].clone());
        }
        assert!(budget_pages > 0);
        assert_eq!(ids.len(), 70);
    }

    #[test]
    fn cursor_rejects_changed_query_snapshot_and_malformed_values() {
        let (_temp, index, browser) = fixture(3, false);
        let first = search_sessions(
            &SearchRequest {
                limit: Some(&json!(1)),
                ..Default::default()
            },
            &index,
            None,
        )
        .unwrap();
        let cursor = &first["next_cursor"];
        let changed = search_sessions(
            &SearchRequest {
                cursor: Some(cursor),
                query: Some(&json!("fixture")),
                ..Default::default()
            },
            &index,
            None,
        )
        .unwrap_err();
        assert_eq!(changed.params()["reason"], "cursor_mismatch");
        // Changing only page size is supported.
        let next = search_sessions(
            &SearchRequest {
                cursor: Some(cursor),
                limit: Some(&json!(2)),
                ..Default::default()
            },
            &index,
            None,
        )
        .unwrap();
        assert_eq!(next["returned"], 2);
        browser.rows.lock().unwrap()[0].insert("updated".into(), json!(200));
        let stale = search_sessions(
            &SearchRequest {
                cursor: Some(cursor),
                ..Default::default()
            },
            &index,
            None,
        )
        .unwrap_err();
        assert_eq!(stale.params()["reason"], "cursor_stale");
        for invalid in [Value::Null, json!(17), json!("garbage")] {
            let error = search_sessions(
                &SearchRequest {
                    cursor: Some(&invalid),
                    ..Default::default()
                },
                &index,
                None,
            )
            .unwrap_err();
            assert_eq!(error.params()["reason"], "cursor_invalid");
        }
    }

    #[test]
    fn incomplete_content_results_are_explicit_lower_bounds() {
        let (_temp, index, _) = fixture(3, false);
        let result = search_sessions(
            &SearchRequest {
                query: Some(&json!("fixture")),
                ..Default::default()
            },
            &index,
            None,
        )
        .unwrap();
        assert_eq!(result["coverage"]["complete"], false);
        assert_eq!(
            result["coverage"]["reasons"],
            json!(["content_index_not_ready"])
        );
        assert_eq!(result["total_matches_relation"], "gte");
        assert_eq!(result["total_matches"], 3);
    }

    #[test]
    fn stale_content_index_cannot_exclude_new_regex_matches() {
        let (temp, index, browser) = fixture(1, false);
        let path = temp.path().join("content.sqlite3");
        let content = Arc::new(ContentIndex::new(Some(path.clone())));
        let records = index.refresh().unwrap();
        assert_eq!(
            content.sync(&index, &records, false).unwrap()["ready"],
            true
        );
        assert!(content
            .sessions_matching_literals(&["newneedle".into()], false)
            .unwrap()
            .unwrap()
            .is_empty());
        // 留住旧索引，让下一次增量写明确失败；避免依赖后台线程调度制造竞态。
        let connection = rusqlite::Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_refresh BEFORE INSERT ON indexed_sessions
             BEGIN SELECT RAISE(ABORT, 'fixture blocks refresh'); END;",
            )
            .unwrap();
        {
            let mut rows = browser.rows.lock().unwrap();
            rows[0].insert("updated".into(), json!(200));
            rows[0].insert(
                "body".into(),
                json!("newneedle only exists in current source"),
            );
        }
        let result = search_sessions(
            &SearchRequest {
                regex: Some(&json!("newneedle")),
                scope: Some(&json!("content")),
                ..Default::default()
            },
            &index,
            Some(&content),
        )
        .unwrap();
        assert_eq!(result["content_index"]["ready"], false);
        assert_eq!(result["content_index"]["pending_sessions"], 1);
        assert_eq!(result["content_index"]["regex_scan"]["mode"], "full");
        assert_eq!(result["content_index"]["regex_scan"]["scanned_sessions"], 1);
        assert_eq!(result["returned"], 1);
        assert_eq!(result["coverage"]["complete"], true);
        assert_eq!(result["total_matches_relation"], "eq");
        assert!(result["sessions"][0]["content_matches"][0]["snippet"]
            .as_str()
            .unwrap()
            .contains("newneedle"));
    }

    #[test]
    fn reordered_agent_filter_has_the_same_cursor_binding() {
        let harness = crate::sessions::index::golden_tests::harness();
        let first = search_sessions(
            &SearchRequest {
                agents: Some(&json!(["grok", "claude"])),
                limit: Some(&json!(1)),
                ..Default::default()
            },
            &harness.index,
            None,
        )
        .unwrap();
        let next = search_sessions(
            &SearchRequest {
                agents: Some(&json!(["claude", "grok"])),
                cursor: Some(&first["next_cursor"]),
                ..Default::default()
            },
            &harness.index,
            None,
        )
        .unwrap();
        assert_eq!(next["returned"], 5);
    }

    #[test]
    fn boolean_operators_require_patterns_or_literal_quotes() {
        for query in ["alpha OR beta", "AND", "alpha NOT beta"] {
            assert_eq!(
                validated_patterns(query, None).unwrap_err().params()["reason"],
                "unsupported_boolean_operator"
            );
        }
        assert!(validated_patterns("", Some(&json!(["alpha OR beta"]))).is_err());
        assert_eq!(
            validated_patterns("alpha \"OR\" beta", None).unwrap(),
            vec!["alpha \"OR\" beta"]
        );
        assert_eq!(
            parse_query_terms("alpha \"OR\" beta"),
            vec!["alpha", "OR", "beta"]
        );
        assert!(validated_patterns("", Some(&json!(["alpha", "beta"]))).is_ok());
    }
}
