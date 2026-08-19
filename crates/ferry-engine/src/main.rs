//! `ferry-engine` 二进制入口。
//!
//! 子命令解析与分支实现都在 [`ferry_engine::server::cli`]，这里只负责把组合根
//! 交给它。

use std::process::ExitCode;

use ferry_engine::bootstrap::build_cli_deps;
use ferry_engine::server::cli;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match cli::main(&argv, build_cli_deps) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
