//! Native fork orchestration. The durable receipt prevents transport retries from
//! creating duplicate sessions. An interrupted pending request is never replayed.
use crate::errors::DomainError;
use crate::operations::types::{EngineError, EngineResult};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

fn key(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn save(path: &Path, value: &Value) -> EngineResult<()> {
    let dir = path.parent().expect("receipt parent");
    std::fs::create_dir_all(dir).map_err(|e| EngineError::runtime(e.to_string()))?;
    let mut temp =
        tempfile::NamedTempFile::new_in(dir).map_err(|e| EngineError::runtime(e.to_string()))?;
    serde_json::to_writer(&mut temp, value).map_err(|e| EngineError::runtime(e.to_string()))?;
    temp.flush()
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|e| EngineError::runtime(e.to_string()))?;
    temp.persist(path)
        .map_err(|e| EngineError::runtime(e.to_string()))?;
    Ok(())
}

pub fn origin(state_dir: &Path, tool: &str, session_id: &str) -> Option<Value> {
    let path = state_dir
        .join("forks/origins")
        .join(key(&format!("{tool}:{session_id}")));
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn create(
    state_dir: &Path,
    request_id: &str,
    through: &str,
    request: Value,
    run: impl FnOnce(Value) -> EngineResult<Value>,
) -> EngineResult<Value> {
    if request_id.is_empty() || request_id.len() > 128 {
        return Err(DomainError::agent_request_invalid("Invalid fork request_id").into());
    }
    let path = state_dir.join("forks/requests").join(key(request_id));
    // Cross-process lock: CLI and desktop can use the same durable request ID.
    std::fs::create_dir_all(path.parent().unwrap())
        .map_err(|e| EngineError::runtime(e.to_string()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|e| EngineError::runtime(e.to_string()))?;
    lock.try_lock()
        .map_err(|_| EngineError::runtime("This fork request is already running"))?;
    let binding = json!([request["tool"], request["sessionId"], through]);
    if path.exists() {
        let receipt: Value = serde_json::from_slice(
            &std::fs::read(&path).map_err(|e| EngineError::runtime(e.to_string()))?,
        )
        .map_err(|e| EngineError::runtime(e.to_string()))?;
        if receipt["binding"] != binding {
            return Err(DomainError::agent_request_invalid(
                "Fork request_id belongs to a different branch point",
            )
            .into());
        }
        return receipt.get("result").cloned().ok_or_else(|| DomainError::agent_request_invalid(
            "Previous fork outcome is uncertain. Refresh native session history before creating another fork.").into());
    }
    save(&path, &json!({"binding": binding, "status": "pending"}))?;
    let result = run(request.clone())?;
    let id = result["sessionId"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != request["sessionId"].as_str().unwrap_or(""))
        .ok_or_else(|| EngineError::runtime("Native fork did not return a new session ID"))?;
    let origin = json!({"tool": request["tool"], "session_id": request["sessionId"],
        "turn": request["turn"], "through": through});
    let outcome =
        json!({"id": id, "tool": request["tool"], "dir": request["cwd"], "origin": origin});
    // Save the successful native identity before optional indexing/UI work.
    save(
        &path,
        &json!({"binding": binding, "status": "created", "result": outcome}),
    )?;
    save(
        &state_dir.join("forks/origins").join(key(&format!(
            "{}:{id}",
            request["tool"].as_str().unwrap_or("")
        ))),
        &origin,
    )?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_request_never_forks_twice_even_after_uncertain_failure() {
        let dir = tempfile::tempdir().unwrap();
        let req = json!({"tool":"pi", "sessionId":"source", "cwd":"/work", "turn":1});
        let first = create(dir.path(), "one", "point", req.clone(), |_| {
            Ok(json!({"sessionId":"child"}))
        })
        .unwrap();
        assert_eq!(
            create(dir.path(), "one", "point", req.clone(), |_| panic!(
                "duplicate"
            ))
            .unwrap(),
            first
        );
        assert!(create(dir.path(), "two", "point", req.clone(), |_| Err(
            EngineError::runtime("timeout")
        ))
        .is_err());
        assert!(create(dir.path(), "two", "point", req, |_| panic!("unsafe retry")).is_err());
    }
}
