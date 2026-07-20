fn main() {
    if std::env::var("PROFILE").as_deref() == Ok("debug")
        && std::env::var_os("TAURI_CONFIG").is_none()
    {
        // externalBin 在 cargo check 时也要求文件存在；开发构建使用源码引擎，不应要求先跑 PyInstaller。
        std::env::set_var(
            "TAURI_CONFIG",
            r#"{"bundle":{"active":false,"externalBin":null}}"#,
        );
    }
    tauri_build::build()
}
