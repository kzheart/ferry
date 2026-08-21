//! ferry-ipc/1 服务端与客户端：请求分发、常驻双线程池、事件帧、socket 传输、
//! CLI 子命令。

pub mod args;
pub mod cli;
pub mod client;
pub mod commands;
pub mod notify;
pub mod rpc;
pub mod serve;
pub mod socket;
