//! Grok 搜索索引的 BLAKE3 内容摘要。
//!
//! 摘要写进 Grok 自己的 `session_search.sqlite`，值必须与 Grok 认的一致，
//! 由本模块的单测（官方向量 + 跨 chunk 边界的冻结向量）守住。

/// `blake3_hex(value)`：32 字节摘要的小写 hex。
pub fn blake3_hex(value: &[u8]) -> String {
    blake3::hash(value).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 官方向量 + 跨 chunk 边界的冻结向量。
    ///
    /// 覆盖 BLAKE3 官方测试向量、1 chunk 以内、正好 1024 字节、跨 chunk
    /// （触发 parent 归并）与含 NUL 的 content_hash 形态。这几个值是冻结基线：
    /// 升级 `blake3` crate 时，任何一条对不上都说明摘要口径变了，Grok 的搜索
    /// 索引会整体失配。
    #[test]
    fn matches_the_frozen_chunk_boundary_vectors() {
        let mut bytes_256: Vec<u8> = Vec::new();
        for _ in 0..4 {
            bytes_256.extend(0u8..=255);
        }
        let cases: Vec<(Vec<u8>, &str)> = vec![
            (
                b"".to_vec(),
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
            ),
            (
                b"abc".to_vec(),
                "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85",
            ),
            (
                b"title\0content".to_vec(),
                "e145d4463ac2b66b19d3ba46968e795a4a450b21fce38906306d2f7c64ee919d",
            ),
            (
                bytes_256,
                "882179b8dbccd285cda241d968cfcccb3156c5edac2fa3761bb6eda7ff8cb172",
            ),
            (
                vec![b'x'; 1023],
                "69a383ad7b84f18e71ef579ff9a766ce75eb90b9e62484dc2a1a01c78f55f03a",
            ),
            (
                vec![b'x'; 1024],
                "71c7a224be567fb9acd2c32f87359835322cf9241b9c01f247fff2b4bdabf644",
            ),
            (
                vec![b'x'; 1025],
                "71b7ce25ca2144dcf4d7ed561d8a526bb7f7adaf8d10a124540d82bf113678c9",
            ),
            (
                vec![b'y'; 4096],
                "90c03fdf2b6c840486a695e15109e1fa77556a38b1afab41518aee66c54f3c9a",
            ),
        ];
        for (case, want) in &cases {
            assert_eq!(&blake3_hex(case), want, "长度 {}", case.len());
        }
    }
}
