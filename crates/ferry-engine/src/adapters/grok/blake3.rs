//! Grok 搜索索引的 BLAKE3 内容摘要。
//!
//! 语义事实源：`engine/adapters/grok/blake3.py`。
//!
//! Python 侧为了避开 PyInstaller onefile 的跨平台成本，手写了一份纯 Python
//! BLAKE3；Rust 侧没有这个约束，直接用 `blake3` crate。两者必须逐字节一致，
//! 由本模块的单测（官方向量 + 与 Python 实现的实跑对照）守住。

/// `blake3_hex(value)`：32 字节摘要的小写 hex。
pub fn blake3_hex(value: &[u8]) -> String {
    blake3::hash(value).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BLAKE3 官方测试向量（与 `tests/test_grok_writer.py` 的断言同源）。
    #[test]
    fn matches_the_official_vectors() {
        assert_eq!(
            blake3_hex(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            blake3_hex(b"abc"),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    /// 与 `engine/adapters/grok/blake3.py` 的纯 Python 实现实跑对照。
    ///
    /// 覆盖 1 chunk 以内、正好 1024 字节、跨 chunk（触发 parent 归并）与含 NUL
    /// 的 content_hash 形态；没有 python3 时跳过（CI 与本机都有）。
    #[test]
    fn matches_the_pure_python_implementation() {
        // 从 crate 目录上溯找到带 `engine/` 的仓库根；找不到（例如在隔离沙箱里
        // 编译）就跳过，官方向量那条用例仍然守住算法本身。
        let mut cursor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        let root = loop {
            if cursor.join("engine/adapters/grok/blake3.py").is_file() {
                break cursor;
            }
            if !cursor.pop() {
                eprintln!("跳过：找不到 Python 参照实现");
                return;
            }
        };
        // 长度覆盖 chunk 边界：1023/1024/1025 与 4096（三层 parent 归并）。
        let script = r#"
import sys
sys.path.insert(0, sys.argv[1])
from engine.adapters.grok.blake3 import blake3_hex
cases = [b"", b"abc", b"title\x00content", bytes(range(256)) * 4,
         b"x" * 1023, b"x" * 1024, b"x" * 1025, b"y" * 4096]
print("\n".join(blake3_hex(case) for case in cases))
"#;
        let output = std::process::Command::new("python3")
            .arg("-c")
            .arg(script)
            .arg(&root)
            .output();
        let Ok(output) = output else {
            eprintln!("跳过：本机没有 python3");
            return;
        };
        assert!(
            output.status.success(),
            "python3 参照实现执行失败: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        let mut bytes_256: Vec<u8> = Vec::new();
        for _ in 0..4 {
            bytes_256.extend(0u8..=255);
        }
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"abc".to_vec(),
            b"title\0content".to_vec(),
            bytes_256,
            vec![b'x'; 1023],
            vec![b'x'; 1024],
            vec![b'x'; 1025],
            vec![b'y'; 4096],
        ];
        assert_eq!(expected.len(), cases.len());
        for (case, want) in cases.iter().zip(&expected) {
            assert_eq!(&blake3_hex(case), want, "长度 {}", case.len());
        }
    }
}
