fn main() {
    if std::env::var("PROFILE").as_deref() == Ok("debug")
        && std::env::var_os("TAURI_CONFIG").is_none()
    {
        // externalBin 在 cargo check 时也要求文件存在；开发构建直接跑仓库里的引擎产物，不应要求先打包 sidecar。
        std::env::set_var(
            "TAURI_CONFIG",
            r#"{"bundle":{"active":false,"externalBin":null}}"#,
        );
    }
    tauri_build::build()
}
