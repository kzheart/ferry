//! 常驻模式：每行一个请求，每行一个响应或事件帧。
//!
//! 硬约束（§2.1 第 3 / 7 条）：
//! - 轻量控制方法进独立的单 worker 池，不受重读队列阻塞；
//! - `PARALLEL_READ_METHOD_NAMES` 的方法进 4-worker 只读池，可乱序完成；
//!   其余请求一律进单 worker 池，严格保序；
//! - 每条输出通道只有一把写锁，响应与事件帧共用，保证行级原子；
//! - 日志只能走 stderr——stdout 上任何杂质都会打死连接；
//! - EOF 后必须等全部在途请求完成再退出；worker 里的失败不能被吞掉。
//!
//! stdio 与本地 socket 是两种传输、**同一组工作道**：[`Lanes`] 由进程共享，
//! 多开一条 socket 连接不会多开一条串行道，否则「mutation 全局串行」会失效。
//! 传输差异全部收在 [`LanePolicy`] 里——socket 的 callers 过滤与 `daemon.*`
//! 拦截就是一条自定义 policy，读线程直接回帧、不进工作道。

use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::contracts::engine_methods::{is_control, is_parallel_read};
use crate::server::notify::{LineWriter, Notifier};

/// 只读池的 worker 数（`MAX_PARALLEL_READS`）。
pub const MAX_PARALLEL_READS: usize = 4;

/// 轻量控制池串行即可；这些方法必须保持常数时间且无 I/O。
const MAX_CONTROL_WORKERS: usize = 1;
const ALLOCATOR_RELIEF_AFTER: Duration = Duration::from_millis(25);

#[cfg(target_os = "macos")]
fn release_unused_allocator_pages() {
    unsafe extern "C" {
        fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
    }
    // null zone requests relief from every malloc zone in the process.
    unsafe {
        malloc_zone_pressure_relief(std::ptr::null_mut(), 0);
    }
}

#[cfg(not(target_os = "macos"))]
fn release_unused_allocator_pages() {}

/// 一次请求的处理函数：拿到原始请求行，返回要写回的 JSON 值。
///
/// `Err` 对齐 Python 里 handler 抛异常的情形：`serve` 收集后在 EOF 处重抛。
pub type ServeHandler = Arc<dyn Fn(&str) -> Result<Value, String> + Send + Sync>;

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct ReliefState {
    active_reads: usize,
    pending_slow: bool,
}

struct ReliefCoordinator {
    state: Mutex<ReliefState>,
    relieve: Arc<dyn Fn() + Send + Sync>,
}

impl ReliefCoordinator {
    fn new(relieve: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ReliefState::default()),
            relieve: Arc::new(relieve),
        })
    }

    fn wrap(self: &Arc<Self>, job: Job) -> Job {
        let coordinator = Arc::clone(self);
        Box::new(move || {
            coordinator.enter();
            let started = Instant::now();
            job();
            coordinator.finish(started.elapsed() >= ALLOCATOR_RELIEF_AFTER);
        })
    }

    fn enter(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .active_reads += 1;
    }

    fn finish(&self, slow: bool) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.pending_slow |= slow;
        state.active_reads = state.active_reads.saturating_sub(1);
        if state.active_reads == 0 && state.pending_slow {
            state.pending_slow = false;
            // 持锁期间新的 read worker 会停在 admission 前；控制 lane 不受影响。
            (self.relieve)();
        }
    }
}

/// 一行请求的处置。
pub enum Lane {
    /// 传输层直接回帧：不经 handler、不占工作道（`daemon.*` 与 callers 拒绝）。
    Immediate(Value),
    /// 进独立轻量控制池。
    Control,
    /// 进 4-worker 只读池。
    Parallel,
    /// 进单 worker 串行池。
    Serial,
}

/// 按原始请求行决定分道方式。
pub type LanePolicy = Arc<dyn Fn(&str) -> Lane + Send + Sync>;

/// 默认分道：契约的 parallel-read 表说了算，其余串行。
pub fn contract_lane_policy() -> LanePolicy {
    Arc::new(|request: &str| match request_method(request).as_deref() {
        Some(method) if is_control(method) => Lane::Control,
        Some(method) if is_parallel_read(method) => Lane::Parallel,
        _ => Lane::Serial,
    })
}

/// 固定大小的线程池（对齐 `ThreadPoolExecutor`）。
struct Pool {
    /// 进程共享，`shutdown` 只能拿 `&self`，所以 sender/workers 都要内部可变。
    sender: Mutex<Option<Sender<Job>>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl Pool {
    fn new(size: usize, name: &str) -> Self {
        let (sender, receiver) = channel::<Job>();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(size);
        for index in 0..size {
            let receiver: Arc<Mutex<Receiver<Job>>> = Arc::clone(&receiver);
            workers.push(
                std::thread::Builder::new()
                    .name(format!("{name}-{index}"))
                    .spawn(move || loop {
                        let job = {
                            let guard = receiver.lock().unwrap_or_else(|error| error.into_inner());
                            guard.recv()
                        };
                        match job {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    })
                    .expect("无法启动 serve worker 线程"),
            );
        }
        Self {
            sender: Mutex::new(Some(sender)),
            workers: Mutex::new(workers),
        }
    }

    fn submit(&self, job: Job) {
        if let Some(sender) = self
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            let _ = sender.send(job);
        }
    }

    /// 等价 `shutdown(wait=True)`：不再接新任务，等在途任务跑完。
    fn shutdown(&self) {
        self.sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        let workers: Vec<JoinHandle<()>> = self
            .workers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .drain(..)
            .collect();
        for worker in workers {
            let _ = worker.join();
        }
    }
}

/// 进程级的三条工作道：1 控制 + 1 串行 + 4 并行读。
pub struct Lanes {
    control: Pool,
    serial: Pool,
    reads: Pool,
    relief: Arc<ReliefCoordinator>,
}

impl Lanes {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            control: Pool::new(MAX_CONTROL_WORKERS, "engine-control"),
            serial: Pool::new(1, "engine-serial"),
            reads: Pool::new(MAX_PARALLEL_READS, "engine-read"),
            relief: ReliefCoordinator::new(release_unused_allocator_pages),
        })
    }

    fn submit(&self, lane: Lane, job: Job) {
        match lane {
            Lane::Control => self.control.submit(job),
            Lane::Parallel => self.reads.submit(self.relief.wrap(job)),
            Lane::Serial => self.serial.submit(job),
            Lane::Immediate(_) => unreachable!("即时帧不得进工作池"),
        }
    }

    /// 关闭所有工作池，并等在途请求结束。
    pub fn shutdown(&self) {
        self.control.shutdown();
        self.reads.shutdown();
        self.serial.shutdown();
    }
}

/// 本次 pump 提交的在途任务计数。
///
/// 共享工作道之后不能再靠「关池子」等自己那批请求：一条连接读到 EOF 时，
/// 别的连接可能正用着同一条道。
#[derive(Default)]
struct InFlight {
    count: Mutex<usize>,
    idle: Condvar,
}

impl InFlight {
    fn enter(&self) {
        *self.count.lock().unwrap_or_else(|error| error.into_inner()) += 1;
    }

    fn leave(&self) {
        let mut count = self.count.lock().unwrap_or_else(|error| error.into_inner());
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.idle.notify_all();
        }
    }

    fn wait_until_idle(&self) {
        let mut count = self.count.lock().unwrap_or_else(|error| error.into_inner());
        while *count > 0 {
            count = self
                .idle
                .wait(count)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

fn request_method(request: &str) -> Option<String> {
    let value: Value = serde_json::from_str(request).ok()?;
    value
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// 一条输出通道的单写者。响应与事件帧共用它，保证行级原子。
fn line_writer(output: Box<dyn Write + Send>, channel: &'static str) -> LineWriter {
    let output = Arc::new(Mutex::new(output));
    Arc::new(move |line: &str| {
        let mut stream = output.lock().unwrap_or_else(|error| error.into_inner());
        // 写失败（对端已断开）不能把引擎主流程打崩，只记 stderr。
        if stream
            .write_all(line.as_bytes())
            .and_then(|()| stream.write_all(b"\n"))
            .and_then(|()| stream.flush())
            .is_err()
        {
            log_warning(&format!("{channel} 写入失败，对端可能已断开"));
        }
    })
}

/// 读循环：逐行分道，返回前等自己提交的任务写完。
fn pump<R: BufRead>(
    input: R,
    write_line: &LineWriter,
    handler: &ServeHandler,
    lanes: &Lanes,
    policy: &LanePolicy,
    failures: &Arc<Mutex<Vec<String>>>,
    pipelined: bool,
) {
    let in_flight = Arc::new(InFlight::default());
    for line in input.lines() {
        let Ok(line) = line else { break };
        let request = line.trim().to_string();
        if request.is_empty() {
            continue;
        }
        let lane = match policy(&request) {
            Lane::Immediate(response) => {
                write_line(&response.to_string());
                continue;
            }
            lane => lane,
        };
        let handler = Arc::clone(handler);
        let write_line = Arc::clone(write_line);
        let failures = Arc::clone(failures);
        let job_in_flight = Arc::clone(&in_flight);
        in_flight.enter();
        lanes.submit(
            lane,
            Box::new(move || {
                match handler(&request) {
                    Ok(response) => write_line(&response.to_string()),
                    Err(error) => failures
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .push(error),
                }
                job_in_flight.leave();
            }),
        );
        if !pipelined {
            // A synchronous Windows named-pipe handle cannot reliably service
            // a worker write while this thread already has the next blocking
            // read outstanding on a duplicate handle. Socket clients are
            // request-response sequential, so finish this frame before reading
            // the next one. Stdio keeps its existing pipelined behavior.
            in_flight.wait_until_idle();
        }
    }
    in_flight.wait_until_idle();
}

/// stdio 常驻：独占一组工作道，EOF 后关池退出。
pub fn serve<R: BufRead>(
    input: R,
    output: Box<dyn Write + Send>,
    handler: ServeHandler,
    notifier: Option<&Notifier>,
) -> Result<(), String> {
    serve_on(Lanes::new(), input, output, handler, notifier)
}

/// stdio 常驻，但工作道由调用方给（App sidecar 与 socket 连接共用同一组）。
pub fn serve_on<R: BufRead>(
    lanes: Arc<Lanes>,
    input: R,
    output: Box<dyn Write + Send>,
    handler: ServeHandler,
    notifier: Option<&Notifier>,
) -> Result<(), String> {
    let write_line = line_writer(output, "stdout");
    if let Some(notifier) = notifier {
        // 事件帧与响应共用同一把输出锁，保证行级原子性。
        notifier.bind(Arc::clone(&write_line));
    }
    let failures: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let policy = contract_lane_policy();
    pump(
        input,
        &write_line,
        &handler,
        &lanes,
        &policy,
        &failures,
        true,
    );
    // EOF：等在途请求结束再退出。
    lanes.shutdown();

    let failures = failures.lock().unwrap_or_else(|error| error.into_inner());
    match failures.first() {
        Some(error) => Err(error.clone()),
        None => Ok(()),
    }
}

/// 一条 socket 连接：共享工作道，连接结束只等自己的在途请求，不关池。
///
/// 事件推送（notifier）不接到 socket：CLI 是请求-响应式调用方，订阅增量事件
/// 会让「一条连接一次调用」的客户端拿到读不完的帧。
pub fn serve_connection<R: BufRead>(
    lanes: &Arc<Lanes>,
    input: R,
    output: Box<dyn Write + Send>,
    handler: ServeHandler,
    policy: LanePolicy,
) -> Result<(), String> {
    let write_line = line_writer(output, "socket");
    let failures: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    pump(
        input,
        &write_line,
        &handler,
        lanes,
        &policy,
        &failures,
        false,
    );
    let failures = failures.lock().unwrap_or_else(|error| error.into_inner());
    match failures.first() {
        Some(error) => Err(error.clone()),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// stderr 日志
// ---------------------------------------------------------------------------

static STDERR_LOGGING: AtomicBool = AtomicBool::new(false);

/// 常驻模式的 `logging.basicConfig(level=INFO, stream=stderr)`。
///
/// 一次性 `rpc` 模式下 Python 没有配置 logging，`lastResort` handler 的阈值是
/// WARNING，所以 INFO 会被丢弃、WARNING 仍进 stderr——这里逐条复刻。
pub fn enable_stderr_logging() {
    STDERR_LOGGING.store(true, Ordering::SeqCst);
}

pub fn stderr_logging_enabled() -> bool {
    STDERR_LOGGING.load(Ordering::SeqCst)
}

pub fn log_info(message: &str) {
    if stderr_logging_enabled() {
        emit_log("INFO", message);
    }
}

pub fn log_warning(message: &str) {
    emit_log("WARNING", message);
}

pub fn log_error(message: &str) {
    emit_log("ERROR", message);
}

fn emit_log(level: &str, message: &str) {
    let _ = writeln!(
        std::io::stderr(),
        "{} {level} ferry_engine.server {message}",
        asctime()
    );
}

/// `%(asctime)s` 的等价物：`YYYY-MM-DD HH:MM:SS,mmm`（UTC）。
fn asctime() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02},{millis:03}",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// 供 [`crate::server::args`] 的反函数测试用。
#[cfg(test)]
pub(crate) fn civil_from_days_for_tests(days: i64) -> (i64, u32, u32) {
    civil_from_days(days)
}

/// Howard Hinnant 的 `civil_from_days`。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;
    use std::time::Duration;

    /// 可跨线程共享的内存输出。
    #[derive(Clone, Default)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuffer {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl SharedBuffer {
        fn lines(&self) -> Vec<Value> {
            let raw = self.0.lock().unwrap().clone();
            String::from_utf8(raw)
                .unwrap()
                // 只按 \n 分行：U+0085 / U+2028 / U+2029 必须原样穿透。
                .split('\n')
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }

    struct ResponseWriter {
        buffer: Vec<u8>,
        responses: mpsc::Sender<Value>,
    }

    impl Write for ResponseWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.buffer.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            let line = std::str::from_utf8(&self.buffer)
                .map_err(std::io::Error::other)?
                .trim();
            if !line.is_empty() {
                let response = serde_json::from_str(line).map_err(std::io::Error::other)?;
                self.responses
                    .send(response)
                    .map_err(std::io::Error::other)?;
            }
            self.buffer.clear();
            Ok(())
        }
    }

    fn request(id: &str, method: &str) -> String {
        format!(r#"{{"protocol":"ferry-ipc/1","id":"{id}","method":"{method}","params":{{}}}}"#)
    }

    #[test]
    fn allocator_relief_runs_once_after_the_entire_read_wave_finishes() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let coordinator = ReliefCoordinator::new(move || {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        for _ in 0..MAX_PARALLEL_READS {
            coordinator.enter();
        }

        coordinator.finish(true);
        coordinator.finish(true);
        coordinator.finish(true);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        // 最后完成的读取本身即使很快，也必须替前面的慢读取收尾。
        coordinator.finish(false);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn jsonl_response_preserves_unicode_line_separators() {
        let content = "alpha\u{85}beta\u{2028}gamma\u{2029}omega";
        let output = SharedBuffer::default();
        let handler: ServeHandler = {
            let content = content.to_string();
            Arc::new(move |_request: &str| {
                Ok(serde_json::json!({
                    "protocol": "ferry-ipc/1",
                    "id": "unicode",
                    "ok": true,
                    "result": {"content": content},
                }))
            })
        };

        serve(
            format!("{}\n", request("unicode", "health")).as_bytes(),
            Box::new(output.clone()),
            handler,
            None,
        )
        .unwrap();

        let records = output.lines();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["result"]["content"], Value::from(content));
    }

    #[test]
    fn parallel_read_requests_can_finish_out_of_input_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let output = SharedBuffer::default();
        let handler: ServeHandler = {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            Arc::new(move |request: &str| {
                let value: Value = serde_json::from_str(request).unwrap();
                let id = value["id"].as_str().unwrap().to_string();
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(if id == "slow" { 80 } else { 10 }));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(serde_json::json!({
                    "protocol": "ferry-ipc/1", "id": id, "ok": true, "result": null,
                }))
            })
        };

        serve(
            format!(
                "{}\n{}\n",
                request("slow", "env"),
                request("fast", "models")
            )
            .as_bytes(),
            Box::new(output.clone()),
            handler,
            None,
        )
        .unwrap();

        assert_eq!(peak.load(Ordering::SeqCst), 2);
        let ids: Vec<String> = output
            .lines()
            .iter()
            .map(|line| line["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["fast", "slow"]);
    }

    #[test]
    fn lookup_requests_do_not_wait_behind_pricing() {
        let output = SharedBuffer::default();
        let handler: ServeHandler = Arc::new(|request: &str| {
            let value: Value = serde_json::from_str(request).unwrap();
            if value["method"] == "pricing" {
                std::thread::sleep(Duration::from_millis(80));
            }
            Ok(serde_json::json!({
                "protocol": "ferry-ipc/1",
                "id": value["id"],
                "ok": true,
                "result": null,
            }))
        });

        serve(
            format!(
                "{}\n{}\n",
                request("pricing", "pricing"),
                request("lookup", "content_search")
            )
            .as_bytes(),
            Box::new(output.clone()),
            handler,
            None,
        )
        .unwrap();

        let ids: Vec<String> = output
            .lines()
            .iter()
            .map(|line| line["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["lookup", "pricing"]);
    }

    #[test]
    fn control_requests_are_not_starved_by_a_saturated_read_pool() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let handler: ServeHandler = {
            let gate = Arc::clone(&gate);
            Arc::new(move |request: &str| {
                let value: Value = serde_json::from_str(request).unwrap();
                if value["method"] == "show" {
                    started_tx.send(()).unwrap();
                    let (released, changed) = gate.as_ref();
                    let mut released = released.lock().unwrap();
                    while !*released {
                        released = changed.wait(released).unwrap();
                    }
                }
                Ok(serde_json::json!({
                    "protocol": "ferry-ipc/1",
                    "id": value["id"],
                    "ok": true,
                    "result": null,
                }))
            })
        };
        let input = (0..MAX_PARALLEL_READS)
            .map(|index| request(&format!("heavy-{index}"), "show"))
            .chain(std::iter::once(request("control", "health")))
            .collect::<Vec<_>>()
            .join("\n");
        let (responses_tx, responses_rx) = mpsc::channel();
        let serving = std::thread::spawn(move || {
            serve(
                std::io::Cursor::new(format!("{input}\n")),
                Box::new(ResponseWriter {
                    buffer: Vec::new(),
                    responses: responses_tx,
                }),
                handler,
                None,
            )
        });

        let all_reads_started = (0..MAX_PARALLEL_READS)
            .all(|_| started_rx.recv_timeout(Duration::from_secs(1)).is_ok());
        let control_response = if all_reads_started {
            responses_rx.recv_timeout(Duration::from_secs(1)).ok()
        } else {
            None
        };

        let (released, changed) = gate.as_ref();
        *released.lock().unwrap() = true;
        changed.notify_all();
        serving.join().unwrap().unwrap();

        assert!(all_reads_started, "4 个重读 worker 未全部进入门控处理器");
        assert_eq!(
            control_response.as_ref().map(|response| &response["id"]),
            Some(&Value::from("control")),
            "health 在重读释放前没有从独立控制通道返回"
        );
    }

    #[test]
    fn non_parallel_requests_stay_on_the_ordered_lane() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let output = SharedBuffer::default();
        let handler: ServeHandler = {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            Arc::new(move |request: &str| {
                let value: Value = serde_json::from_str(request).unwrap();
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(20));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(serde_json::json!({
                    "protocol": "ferry-ipc/1", "id": value["id"], "ok": true, "result": null,
                }))
            })
        };

        serve(
            format!(
                "{}\n{}\n",
                request("first", "scan"),
                request("second", "scan")
            )
            .as_bytes(),
            Box::new(output.clone()),
            handler,
            None,
        )
        .unwrap();

        assert_eq!(peak.load(Ordering::SeqCst), 1);
        let ids: Vec<String> = output
            .lines()
            .iter()
            .map(|line| line["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, ["first", "second"]);
    }

    #[test]
    fn worker_failures_are_not_swallowed() {
        let handler: ServeHandler = Arc::new(|_request: &str| Err("worker failed".to_string()));
        let error = serve(
            format!("{}\n", request("failure", "health")).as_bytes(),
            Box::new(SharedBuffer::default()),
            handler,
            None,
        )
        .unwrap_err();
        assert_eq!(error, "worker failed");
    }

    #[test]
    fn event_frames_share_the_output_lock() {
        let output = SharedBuffer::default();
        let notifier = Notifier::new();
        let handler: ServeHandler = Arc::new(|_request: &str| {
            Ok(serde_json::json!({
                "protocol": "ferry-ipc/1", "id": "x", "ok": true, "result": null,
            }))
        });
        serve(
            format!("{}\n", request("x", "health")).as_bytes(),
            Box::new(output.clone()),
            handler,
            Some(&notifier),
        )
        .unwrap();
        // serve 结束后 notifier 仍绑着同一把锁，事件帧照样成行写出。
        notifier
            .emit("sessions.changed", serde_json::json!({"generation": 1}))
            .unwrap();

        let lines = output.lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].get("id").is_none());
        assert_eq!(lines[1]["type"], Value::from("sessions.changed"));
    }

    #[test]
    fn asctime_formats_like_python_logging() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        let stamp = asctime();
        assert_eq!(stamp.len(), 23, "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[19..20], ",");
    }
}
