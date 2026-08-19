//! 正则检索：必然字面量预过滤 + 原始转录扫描。
//!
//! trigram 索引无法加速任意正则，但绝大多数实用正则含有必然出现的字面片段
//! （`ghp_[A-Za-z0-9]{36}` 必含 `ghp_`）。提取这些片段打到 FTS 索引缩小候选集，
//! 再对候选会话的**原始转录**跑正则引擎——原文没有 16 KB 截断。
//!
//! Python 用 `re._parser` 的节点树做提取，Rust 用 `regex-syntax` 的 HIR：
//! 「必经串接路径」这条主语义逐条对齐（见 [`required_literals`] 的分支注释），
//! 但两个解析器不同源，少数 pattern 上提取结果不同。**分歧只改变候选集的宽窄
//! 与 `regex_scan` 的覆盖度数字，不产生漏报**；逐条钉在
//! `documented_divergences_from_the_python_extractor` 测试里。
//!
//! **已知差异**：Python `re` 支持回溯特性（后向引用、环视），Rust `regex` 不
//! 支持，这类 pattern 在 Rust 侧会落到 `regex 无法编译` 的 AgentRequestError。

use regex::Regex;
use regex_syntax::hir::{Class, ClassUnicode, ClassUnicodeRange, Hir, HirKind};
use serde_json::{Map, Value};

use crate::errors::{DomainError, DomainResult};
use crate::model::{tool_result_text, BlockKind, Message, Session};

/// 与 `content_index::MIN_TRIGRAM_CHARS` 同源：短于 3 字符走不了 trigram。
const MIN_LITERAL_CHARS: usize = 3;
const SNIPPET_BEFORE: usize = 120;
const SNIPPET_AFTER: usize = 240;
const MATCHES_PER_SESSION: usize = 3;

fn regex_error(message: impl Into<String>) -> DomainError {
    let mut params = Map::new();
    params.insert("field".into(), Value::from("regex"));
    DomainError::new(
        "agent.request_invalid",
        "AgentRequestError",
        message,
        params,
    )
}

pub fn compile_regex(pattern: Option<&Value>) -> DomainResult<Regex> {
    let text = pattern
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| regex_error("regex 必须是非空字符串"))?;
    if text.chars().count() > 500 {
        return Err(regex_error("regex 不能超过 500 字符"));
    }
    Regex::new(text).map_err(|error| regex_error(format!("regex 无法编译: {error}")))
}

/// 提取正则里必然出现的字面片段（≥3 字符），供 trigram 预过滤。
///
/// 提取是保守的：只收必经串接路径上的连续字面量；分支、可选重复、字符类、
/// 环视一律不贡献，只作为字面量运行的边界。提不出来返回空表，调用方退化为
/// 全量扫描——预过滤只许缩小候选集，绝不许制造漏报。
pub fn required_literals(pattern: &str) -> Vec<String> {
    let Ok(hir) = regex_syntax::Parser::new().parse(pattern) else {
        return Vec::new();
    };
    let mut literals = Vec::new();
    walk(&hir, &mut literals);
    literals
}

/// 进入一个新的「节点序列」上下文：Python 的 `walk(nodes)` 每次都新建 run。
fn walk(node: &Hir, literals: &mut Vec<String>) {
    let mut run = String::new();
    visit(node, &mut run, literals);
    flush(&mut run, literals);
}

fn flush(run: &mut String, literals: &mut Vec<String>) {
    if run.chars().count() >= MIN_LITERAL_CHARS {
        literals.push(run.clone());
    }
    run.clear();
}

fn visit(node: &Hir, run: &mut String, literals: &mut Vec<String>) {
    match node.kind() {
        // 连续字面量：HIR 已把相邻字符合并成一段字节串。
        HirKind::Literal(literal) => match std::str::from_utf8(&literal.0) {
            Ok(text) => run.push_str(text),
            // 非 UTF-8 字面量（字节模式）不参与 trigram 预过滤。
            Err(_) => flush(run, literals),
        },
        // 串接：与 Python 的 `for op, arg in nodes` 同层，共享同一个 run。
        HirKind::Concat(items) => {
            for item in items {
                visit(item, run, literals);
            }
        }
        // SUBPATTERN：先 flush 再以新 run 递归。
        HirKind::Capture(capture) => {
            flush(run, literals);
            walk(&capture.sub, literals);
        }
        // MAX_REPEAT / MIN_REPEAT：先 flush；下界 ≥1 时内容才是必经的。
        HirKind::Repetition(repetition) => {
            flush(run, literals);
            if repetition.min >= 1 {
                walk(&repetition.sub, literals);
            }
        }
        // `(?i)` 下的每个字符在 HIR 里是折叠类而不是 Literal，还原成字面量。
        HirKind::Class(class) => match folded_char(class) {
            Some(character) => run.push(character),
            // 真正的字符类（IN）：不贡献，只作为运行边界。
            None => flush(run, literals),
        },
        // BRANCH / ANY / AT / ASSERT…：不贡献，只作为运行边界。
        _ => flush(run, literals),
    }
}

/// 若一个字符类恰好是**某个单字符的简单大小写折叠闭包**，还原出那个字符。
///
/// Python 的 `re._parser` 不把 `(?i)` 下沉到节点上，`(?i)abcdef` 仍是一串
/// LITERAL，于是 `required_literals` 给出 `["abcdef"]`；regex-syntax 的 HIR 会
/// 把每个字符翻成折叠类，不还原的话预过滤对所有大小写不敏感的正则都退化成全量
/// 扫描。（无 `(?i)` 的单字符类如 `[c]` 在 `Hir::class` 构造期就已塌成 Literal，
/// 走不到这里。）
///
/// **已知漏报面**：折叠闭包可能含非 ASCII 成员（`k` 的闭包含 U+212A KELVIN
/// SIGN），而 FTS5 trigram 只折叠 ASCII 大小写，含这些字符的文档会被预过滤
/// 挡掉。这里刻意不"修得更对"：预过滤口径与索引口径一旦分叉，搜索上报的
/// 覆盖度字段就不再成立。
fn folded_char(class: &Class) -> Option<char> {
    let Class::Unicode(unicode) = class else {
        // 字节类只在 `(?-u)` 字节模式下出现，不参与 trigram 预过滤。
        return None;
    };
    let candidate = unicode.ranges().first()?.start();
    let mut probe = ClassUnicode::new([ClassUnicodeRange::new(candidate, candidate)]);
    probe.case_fold_simple();
    (probe.ranges() == unicode.ranges()).then(|| {
        // 折叠类的首个区间起点通常是大写形态；小写更贴近 pattern 的书写习惯，
        // 且下游 trigram 匹配本就大小写不敏感，取哪个形态结果相同。
        candidate.to_lowercase().next().unwrap_or(candidate)
    })
}

/// 与 `content_index` 的 `extract` 同口径抽正文/工具输出，但**不截断**。
fn sources(message: &Message, include_tool_outputs: bool) -> Vec<String> {
    let mut texts: Vec<&str> = Vec::new();
    let mut tools: Vec<String> = Vec::new();
    for block in &message.blocks {
        if block.kind == BlockKind::Text && !block.text.is_empty() {
            texts.push(&block.text);
        } else if include_tool_outputs && block.kind == BlockKind::Tool {
            let Some(call) = block.tool.as_ref() else {
                continue;
            };
            tools.push(format!("[tool {}]", call.name));
            let output = tool_result_text(call.result.as_ref());
            if !output.is_empty() {
                tools.push(output);
            }
        }
    }
    let mut result = Vec::new();
    if !texts.is_empty() {
        result.push(texts.join("\n"));
    }
    if !tools.is_empty() {
        result.push(tools.join("\n"));
    }
    result
}

fn snippet(source: &str, start: usize, end: usize) -> String {
    // Python 的 match.start()/end() 是**字符**下标，切片同样按字符。
    let start = source[..start].chars().count();
    let end = source[..end].chars().count();
    let total = source.chars().count();
    let left = start.saturating_sub(SNIPPET_BEFORE);
    let right = total.min(end + SNIPPET_AFTER);
    format!(
        "{}{}{}",
        if left > 0 { "…" } else { "" },
        super::agent_read::char_slice(source, left, right),
        if right < total { "…" } else { "" }
    )
}

/// 对一个会话的原始消息跑正则；返回 (命中消息数, 至多 3 条命中行)。
///
/// message/turn 编号与 `content_index` 完全同口径，命中可直接交给
/// `session_read from_message` 跳读。
pub fn scan_session(
    session: &Session,
    compiled: &Regex,
    include_tool_outputs: bool,
) -> (i64, Vec<Map<String, Value>>) {
    let mut count = 0i64;
    let mut rows: Vec<Map<String, Value>> = Vec::new();
    let mut turn = 0i64;
    for (message_index, message) in session.messages.iter().enumerate() {
        if message.role == "user" {
            turn += 1;
        }
        let mut hit: Option<(String, usize, usize)> = None;
        for source in sources(message, include_tool_outputs) {
            if let Some(found) = compiled.find(&source) {
                hit = Some((source.clone(), found.start(), found.end()));
                break;
            }
        }
        let Some((source, start, end)) = hit else {
            continue;
        };
        count += 1;
        if rows.len() < MATCHES_PER_SESSION {
            let mut row = Map::new();
            row.insert("message".into(), Value::from(message_index as i64 + 1));
            row.insert("turn".into(), Value::from(turn));
            row.insert("role".into(), Value::from(message.role.as_str()));
            row.insert("snippet".into(), Value::from(snippet(&source, start, end)));
            rows.push(row);
        }
    }
    (count, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Block;
    use serde_json::json;

    /// 期望值与 Python `regex_search.required_literals` 逐条对照。
    #[test]
    fn literals_only_come_from_mandatory_concatenation_paths() {
        assert_eq!(required_literals("ghp_[A-Za-z0-9]{36}"), vec!["ghp_"]);
        assert_eq!(required_literals("abcdef"), vec!["abcdef"]);
        // 分支不贡献。
        assert!(required_literals("(?:foo|barbaz)").is_empty());
        // 可选重复不贡献；下界 ≥1 的重复内容才算必经。
        assert!(required_literals("(?:abcd)*").is_empty());
        assert_eq!(required_literals("(?:abcd)+"), vec!["abcd"]);
        // 捕获组是运行边界：abc 与 def 分成两段。
        assert_eq!(required_literals("abc(def)"), vec!["abc", "def"]);
        // 短于 3 字符的片段丢弃。
        assert!(required_literals("ab[0-9]cd").is_empty());
        // 字符类/锚点是边界。
        assert_eq!(required_literals("^token=[0-9]+$"), vec!["token="]);
        // 无法解析的 pattern 退化为空表。
        assert!(required_literals("(").is_empty());
    }

    /// `(?i)` 下的字符在 HIR 里是折叠类；不还原的话所有大小写不敏感的正则都
    /// 退化成全量扫描。
    #[test]
    fn case_insensitive_literals_survive_the_hir_class_encoding() {
        // 统一取小写形态（下游 trigram 大小写不敏感）。
        assert_eq!(required_literals("(?i)abcdef"), vec!["abcdef"]);
        assert_eq!(required_literals("(?i:abcdef)"), vec!["abcdef"]);
        // 大小写不敏感时书写形态无关，统一折成小写。
        assert_eq!(required_literals("(?i)ABC_def"), vec!["abc_def"]);
        assert_eq!(required_literals(r"(?i)\d{3}abcdef"), vec!["abcdef"]);
        // 折叠闭包含非 ASCII 成员的字符（k → U+212A）同样还原成字面量。
        assert_eq!(required_literals("(?i)task_key"), vec!["task_key"]);
        // 真正的字符类仍是边界，不会被误当字面量。
        assert!(required_literals("ab[0-9]cd").is_empty());
        assert_eq!(required_literals("abc[A-Z]def"), vec!["abc", "def"]);
    }

    /// **刻意登记的与 Python 的分歧**：两侧的必然字面量提取器不同源
    /// （CPython `re._parser` vs regex-syntax HIR），下面这些 pattern 上结果不
    /// 同。三条都只影响预过滤的候选集宽窄与 `regex_scan` 的覆盖度数字，**不会
    /// 产生漏报**（Rust 侧要么更保守、要么是更强的必经字面量）。
    #[test]
    fn documented_divergences_from_the_python_extractor() {
        // 1) 分支公共前缀：CPython 的解析器会把 `abc|abcdef` 折成
        //    `LITERAL a,b,c + BRANCH`，于是给出 ['abc']；regex-syntax 的
        //    `lift_common_prefix` 要求所有分支都是 Concat，这里保持 Alternation。
        //    Rust 更保守 → 退化为全量扫描。
        assert!(required_literals("abc|abcdef").is_empty());
        // 2) 相邻字面量跨节点合并：Python 给 ['abcd', 'efg']，
        //    regex-syntax 的 `Hir::concat` 把 `(?:d|d)` 塌成单字符后吞成一整段。
        //    Rust 的候选集更窄，仍是必经字面量。
        assert_eq!(required_literals("abc(?:d|d)efg"), vec!["abcdefg"]);
        // 3) 单字符类：两侧都当字面量（`Hir::class` 构造期就塌成 Literal）。
        assert_eq!(required_literals("ab[c]def"), vec!["abcdef"]);
        // 4) 手写的大小写对 `[Ff]` 与 `(?i)f` 在 HIR 里是同一个类，Rust 一并还原；
        //    Python 把它当 IN 处理（边界）。Rust 的候选集更窄，仍无漏报——下游
        //    trigram 大小写不敏感，搜 `f` 同样命中 `F`。
        assert_eq!(required_literals("ab[Ff]cd"), vec!["abfcd"]);
    }

    #[test]
    fn compile_regex_validates_its_input() {
        assert!(compile_regex(Some(&json!("a.b"))).is_ok());
        assert!(compile_regex(Some(&json!(""))).is_err());
        assert!(compile_regex(Some(&json!("   "))).is_err());
        assert!(compile_regex(None).is_err());
        assert!(compile_regex(Some(&json!(1))).is_err());
        let long = compile_regex(Some(&json!("a".repeat(501)))).unwrap_err();
        assert_eq!(long.message(), "regex 不能超过 500 字符");
        let broken = compile_regex(Some(&json!("("))).unwrap_err();
        assert!(broken.message().starts_with("regex 无法编译"));
        assert_eq!(broken.params()["field"], Value::from("regex"));
    }

    fn message(role: &str, text: &str) -> Message {
        let mut message = Message::new(role);
        message.blocks.push(Block::text(text));
        message
    }

    #[test]
    fn scan_session_numbers_messages_and_caps_rows() {
        let mut session = Session::new("claude", "s", "/tmp");
        session.messages = vec![
            message("user", "hello ghp_aaa"),
            message("assistant", "nothing here"),
            message("user", "ghp_bbb"),
            message("assistant", "ghp_ccc"),
            message("assistant", "ghp_ddd"),
        ];
        let compiled = Regex::new("ghp_[a-z]{3}").unwrap();
        let (count, rows) = scan_session(&session, &compiled, false);
        assert_eq!(count, 4);
        assert_eq!(rows.len(), MATCHES_PER_SESSION);
        assert_eq!(rows[0]["message"], Value::from(1));
        assert_eq!(rows[0]["turn"], Value::from(1));
        assert_eq!(rows[1]["message"], Value::from(3));
        assert_eq!(rows[1]["turn"], Value::from(2));
        assert_eq!(rows[2]["role"], Value::from("assistant"));
    }

    #[test]
    fn snippets_are_windowed_by_characters() {
        let source = format!("{}中文命中{}", "a".repeat(400), "b".repeat(400));
        let compiled = Regex::new("中文命中").unwrap();
        let found = compiled.find(&source).unwrap();
        let text = snippet(&source, found.start(), found.end());
        assert!(text.starts_with('…') && text.ends_with('…'));
        assert!(text.contains("中文命中"));
        // 前 120 + 命中 4 + 后 240 + 两个省略号。
        assert_eq!(text.chars().count(), 120 + 4 + 240 + 2);
    }
}
