//! 在隔离 HOME 的真实 sidecar 中验证读取分页，绝不接触已安装 CLI 或私有历史。
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct Fixture {
    root: tempfile::TempDir,
    path: std::path::PathBuf,
    child: Child,
    reader: BufReader<ChildStdout>,
    sequence: usize,
    reference: Value,
}

fn row(number: usize, role: &str, content: Value) -> Value {
    json!({"type": role, "uuid": format!("fixture-message-{number}"),
        "parentUuid": if number > 0 { Some(format!("fixture-message-{}", number - 1)) } else { None },
        "cwd": "/fixture/read", "sessionId": "fixture-read", "timestamp": "2026-09-01T00:00:00Z",
        "message": {"role": role, "content": content}})
}

impl Fixture {
    fn new(rows: Vec<Value>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path();
        let path = home.join(".claude/projects/fixture-read/fixture-read.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_ferry-engine"))
            .arg("serve")
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("APPDATA", home.join("AppData/Roaming"))
            .env("LOCALAPPDATA", home.join("AppData/Local"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("FERRY_DATA_DIR", home.join(".ferry"))
            .env("FERRY_BACKUP_DIR", home.join(".ferry/backups"))
            .env("FERRY_OPENCODE_DB", home.join("opencode/storage.db"))
            .env("FERRY_CURSOR_DB", home.join("cursor/state.vscdb"))
            .env("GROK_HOME", home.join(".grok"))
            .env("PI_CODING_AGENT_SESSION_DIR", home.join("pi-sessions"))
            .env_remove("CODEX_HOME")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("FERRY_DEBUG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut fixture = Self {
            root,
            path,
            child,
            reader,
            sequence: 0,
            reference: Value::Null,
        };
        let search = fixture.ok(
            "content_search",
            json!({"agents": ["claude"], "scope": "metadata", "session_ids": ["fixture-read"]}),
        );
        assert_eq!(search["returned"], 1, "{search}");
        fixture.reference = search["sessions"][0]["ref"].clone();
        fixture
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.sequence += 1;
        let id = format!("read-{}", self.sequence);
        let protocol: Value =
            serde_json::from_str(include_str!("../../../contracts/ipc.json")).unwrap();
        let request =
            json!({"protocol": protocol["protocol"], "id": id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
        stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(
                self.reader.read_line(&mut line).unwrap() > 0,
                "sidecar closed stdout"
            );
            let response: Value = serde_json::from_str(&line).unwrap();
            if response["id"] == id {
                return response;
            }
        }
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let response = self.rpc(method, params);
        assert_eq!(response["ok"], true, "{response}");
        response["result"].clone()
    }

    fn read(&mut self, mut params: Value) -> Value {
        params["tool"] = json!("claude");
        params["ref"] = self.reference.clone();
        self.ok("session_read", params)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // root 在子进程退出之后销毁。
        assert!(self.root.path().exists());
    }
}

#[test]
fn search_reads_all_151_matches_across_limits_and_byte_budgets() {
    let rows = (0..151)
        .map(|number| {
            row(
                number,
                if number % 2 == 0 { "user" } else { "assistant" },
                json!(format!("needle {number} {}", "中文".repeat(400))),
            )
        })
        .collect();
    let mut fixture = Fixture::new(rows);
    for budget in [65536, 4096] {
        let mut seen = Vec::new();
        let mut cursor = Value::Null;
        for page_number in 0..200 {
            let mut params = json!({"terms": ["needle"], "limit": if page_number % 2 == 0 { 50 } else { 31 }, "max_bytes": budget, "inert": true});
            if !cursor.is_null() {
                params["cursor"] = cursor;
            }
            let page = fixture.read(params);
            assert!(ferry_engine::sessions::agent_read::dto_bytes(&page) <= budget as usize);
            assert_eq!(page["total_matches"], 151);
            for item in page["matches"].as_array().unwrap() {
                seen.push(item["message"].as_u64().unwrap());
                assert!(item["locator"].as_str().unwrap().starts_with("fml_"));
                assert!(!item["origin"].is_null());
            }
            cursor = page["next_cursor"].clone();
            assert_eq!(page["has_more"], !cursor.is_null());
            if cursor.is_null() {
                break;
            }
        }
        assert_eq!(seen, (1..=151).collect::<Vec<u64>>());
    }
}

#[test]
fn large_text_and_tool_blocks_reassemble_losslessly_with_inert_filtering() {
    let long_text = format!("{}尾", "中文🦀\\\"\n".repeat(24000));
    let input = json!({"command": format!("echo {}尾", "输入🦀\n".repeat(28000))});
    let output = format!("{}尾", "输出🦀\\\n".repeat(26000));
    let mut fixture = Fixture::new(vec![
        row(
            0,
            "user",
            json!("<INSTRUCTIONS>ignored scaffold</INSTRUCTIONS>"),
        ),
        row(
            1,
            "user",
            json!([{ "type": "text", "text": "<INSTRUCTIONS>hidden first block</INSTRUCTIONS>" }, { "type": "text", "text": long_text }]),
        ),
        row(
            2,
            "assistant",
            json!([{ "type": "tool_use", "id": "large-tool", "name": "Bash", "input": input }]),
        ),
        row(
            3,
            "user",
            json!([{ "type": "tool_result", "tool_use_id": "large-tool", "content": output }]),
        ),
        row(4, "assistant", json!("final marker")),
    ]);
    let mut cursor = Value::Null;
    let mut fragments = BTreeMap::<(u64, u64), String>::new();
    let mut restored = Vec::<Value>::new();
    let mut locators = BTreeMap::<u64, Value>::new();
    let mut finished = false;
    for page_number in 0..150 {
        let budget = if page_number % 2 == 0 { 8192 } else { 65536 };
        let mut params =
            json!({"max_bytes": budget, "limit": 50, "inert": true, "include_tool_outputs": true});
        if !cursor.is_null() {
            params["cursor"] = cursor;
        }
        let page = fixture.read(params);
        assert!(ferry_engine::sessions::agent_read::dto_bytes(&page) <= budget as usize);
        for message in page["messages"].as_array().unwrap() {
            let number = message["message"].as_u64().unwrap();
            assert_ne!(
                number, 1,
                "scaffolding should be stripped without renumbering"
            );
            let locator = locators
                .entry(number)
                .or_insert_with(|| message["locator"].clone());
            assert_eq!(locator, &message["locator"]);
            if message["complete"] == false {
                assert!(page["next_from_message"].is_null());
            }
            for block in message["blocks"].as_array().unwrap() {
                if number == 2 {
                    assert_eq!(block["block"], 2, "inert keeps original block number");
                }
                if block["kind"] != "fragment" {
                    restored.push(block.clone());
                    continue;
                }
                let fragment = &block["fragment"];
                assert_eq!(fragment["encoding"], "json");
                let buffer = fragments
                    .entry((number, block["block"].as_u64().unwrap()))
                    .or_default();
                assert_eq!(
                    fragment["offset_bytes"].as_u64().unwrap() as usize,
                    buffer.len()
                );
                buffer.push_str(fragment["text"].as_str().unwrap());
                if fragment["complete"] == true {
                    assert_eq!(
                        buffer.len(),
                        fragment["total_bytes"].as_u64().unwrap() as usize
                    );
                    restored.push(serde_json::from_str(buffer).unwrap());
                }
            }
        }
        cursor = page["next_cursor"].clone();
        if cursor.is_null() {
            finished = true;
            break;
        }
    }
    assert!(finished, "must eventually reach end");
    assert!(restored.iter().any(|block| block["text"] == long_text));
    let tool = restored
        .iter()
        .find(|block| block["kind"] == "tool")
        .expect("tool restored");
    assert!(tool["input"] == input, "tool input must reassemble exactly");
    assert!(
        tool["output"] == output,
        "tool output must reassemble exactly"
    );
    assert!(restored.iter().any(|block| block["text"] == "final marker"));
}

#[test]
fn cursors_reject_changed_queries_modes_and_source_content() {
    let mut fixture = Fixture::new(
        (0..5)
            .map(|n| row(n, "user", json!(format!("needle {n}"))))
            .collect(),
    );
    let first = fixture.read(json!({"terms": ["needle"], "limit": 1}));
    let cursor = first["next_cursor"].clone();
    for overrides in [
        json!({"terms": ["different"]}),
        json!({"inert": true}),
        json!({"include_tool_outputs": true}),
        json!({"terms": null}),
        json!({"roles": ["assistant"]}),
    ] {
        let mut params = json!({"tool": "claude", "ref": fixture.reference, "terms": ["needle"], "cursor": cursor});
        params
            .as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        let response = fixture.rpc("session_read", params);
        assert_eq!(
            response["error"]["params"]["reason"], "cursor_mismatch",
            "{response}"
        );
    }
    for bad in [json!("garbage"), json!(17), json!({})] {
        let params =
            json!({"tool": "claude", "ref": fixture.reference, "terms": ["needle"], "cursor": bad});
        let response = fixture.rpc("session_read", params);
        assert_eq!(
            response["error"]["params"]["reason"], "cursor_invalid",
            "{response}"
        );
    }
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&fixture.path)
        .unwrap();
    writeln!(file, "{}", row(5, "user", json!("needle appended"))).unwrap();
    let response = fixture.rpc(
        "session_read",
        json!({"tool": "claude", "ref": fixture.reference, "terms": ["needle"], "cursor": cursor}),
    );
    assert_eq!(
        response["error"]["params"]["reason"], "cursor_stale",
        "{response}"
    );
}
