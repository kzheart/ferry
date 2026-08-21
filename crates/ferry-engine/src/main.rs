//! `ferry-engine` 二进制入口。
//!
//! 子命令解析与分支实现都在 [`ferry_engine::server::cli`]，这里只负责把组合根
//! 交给它，并把退出码原样交给内核。
//!
//! 退出码是薄客户端输出契约的一部分：成功 0、引擎业务错误 1、连接/传输失败 2、
//! 等待超时 3。用法错误与维护者子命令的失败沿用 1。

use std::process::ExitCode;

use ferry_engine::bootstrap::build_cli_deps;
use ferry_engine::server::cli;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match cli::main(&argv, build_cli_deps) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
