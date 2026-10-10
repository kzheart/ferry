//! 会话读取参数及其游标身份。页大小和字节预算允许在续读时调整。
use super::read_cursor::digest;
use super::safety::{bounded_int, MAX_AGENT_DTO_BYTES};
use crate::errors::DomainResult;
use serde_json::{json, Value};

pub(super) const MAX_RESULTS: i64 = 50;
pub(super) const MAX_MESSAGES: i64 = 50;
pub(super) const MAX_BYTES: i64 = 64 * 1024;
pub(super) const DEFAULT_BYTES: i64 = 24 * 1024;

pub(super) struct ContextOptions {
    pub first: usize,
    pub count: usize,
    pub budget: usize,
}

impl ContextOptions {
    pub fn parse(
        from: Option<&Value>,
        limit: Option<&Value>,
        bytes: Option<&Value>,
    ) -> DomainResult<Self> {
        Ok(Self {
            first: bounded_int(from, 1, 1, 1_000_000, "from_message")? as usize,
            count: bounded_int(limit, 20, 1, MAX_MESSAGES, "limit")? as usize,
            budget: bounded_int(bytes, DEFAULT_BYTES, 1024, MAX_BYTES, "max_bytes")? as usize,
        })
    }
}

pub(super) struct SearchOptions {
    pub count: usize,
    pub budget: usize,
}

impl SearchOptions {
    pub fn parse(limit: Option<&Value>, bytes: Option<&Value>) -> DomainResult<Self> {
        Ok(Self {
            count: bounded_int(limit, 20, 1, MAX_RESULTS, "limit")? as usize,
            budget: bounded_int(
                bytes,
                MAX_AGENT_DTO_BYTES as i64,
                1024,
                MAX_BYTES,
                "max_bytes",
            )? as usize,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct ReadScope<'a> {
    pub tool: &'a str,
    pub reference: &'a str,
    pub inert: bool,
    pub outputs: bool,
    pub through: Option<&'a str>,
}

impl ReadScope<'_> {
    pub fn context_binding(&self, first: usize) -> String {
        digest(&json!([
            "context",
            self.tool,
            self.reference,
            first,
            self.inert,
            self.outputs,
            self.through
        ]))
    }

    pub fn search_binding(&self, terms: &[String], roles: &[String]) -> String {
        digest(&json!([
            "search",
            self.tool,
            self.reference,
            terms,
            roles,
            self.inert,
            self.outputs,
            self.through
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_defaults_and_adjustable_page_sizes() {
        let defaults = ContextOptions::parse(None, None, None).unwrap();
        assert_eq!(
            (defaults.first, defaults.count, defaults.budget),
            (1, 20, 24576)
        );
        let adjusted =
            ContextOptions::parse(Some(&json!(3)), Some(&json!(50)), Some(&json!(4096))).unwrap();
        assert_eq!(
            (adjusted.first, adjusted.count, adjusted.budget),
            (3, 50, 4096)
        );
    }

    #[test]
    fn options_reject_invalid_types() {
        for value in [json!("4096"), json!(true), json!(-1), json!(65537)] {
            assert!(ContextOptions::parse(None, None, Some(&value)).is_err());
            assert!(SearchOptions::parse(None, Some(&value)).is_err());
        }
    }
}
