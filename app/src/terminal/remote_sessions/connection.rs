use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_process::{Child, ChildStdin, ChildStdout};
use command::r#async::Command;
use futures::channel::{mpsc, oneshot};
use futures::io::{AsyncReadExt, AsyncWriteExt};
use futures::lock::Mutex;
use futures::SinkExt;
use warpui::r#async::executor::{Background, BackgroundTask};
use warpui::r#async::{FutureExt as _, Timer};

use crate::settings::remote_hosts::RemoteHost;
use crate::terminal::remote_sessions::cc_parser::{CcStream, ControlEvent};
use crate::terminal::remote_sessions::commands::{
    heartbeat_cmd, list_sessions_cmd, parse_sessions,
};
use crate::terminal::remote_sessions::probe::classify_ssh_error;
use crate::terminal::remote_sessions::ssh_args::target_args;
use crate::terminal::remote_sessions::types::{HostError, RemoteTmuxSession};

const MASTER_PROBE_ATTEMPTS: u32 = 20;
const MASTER_PROBE_DELAY: Duration = Duration::from_millis(250);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ConnectionConfig {
    pub heartbeat_interval: Duration,
}

#[derive(Debug)]
pub enum ConnectionEvent {
    Sessions(Vec<RemoteTmuxSession>),
    Exit(Option<String>),
    Error(HostError),
}

enum Pending {
    UserCommand(oneshot::Sender<Result<Vec<String>, String>>),
    Refresh,
}

type PendingQueue = Arc<Mutex<VecDeque<Pending>>>;

pub struct RemoteHostConnection {
    pub socket_path: PathBuf,
    host: String,
    child: Arc<Mutex<Option<Child>>>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: PendingQueue,
    closing: Arc<AtomicBool>,
    _reader_task: BackgroundTask,
    _heartbeat_task: BackgroundTask,
}

impl RemoteHostConnection {
    pub async fn open(
        host: &RemoteHost,
        socket_path: PathBuf,
        events_tx: mpsc::UnboundedSender<ConnectionEvent>,
        executor: Arc<Background>,
        config: ConnectionConfig,
    ) -> Result<Self, HostError> {
        ensure_socket_dir(&socket_path).map_err(|e| HostError::Other(e.to_string()))?;
        ensure_master(host, &socket_path).await?;

        let mut cmd = Command::new("ssh");
        cmd.arg("-tt")
            .arg("-o")
            .arg("ControlMaster=no")
            .arg("-o")
            .arg(format!("ControlPath={}", socket_path.display()))
            .arg("-o")
            .arg("BatchMode=yes")
            .args(target_args(host))
            .arg("tmux -CC new-session -A -s __warp_ctrl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| HostError::Other(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or(HostError::TmuxFailedToStart(String::new()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(HostError::TmuxFailedToStart(String::new()))?;

        let pending: PendingQueue = Arc::new(Mutex::new(VecDeque::new()));
        let stdin_holder = Arc::new(Mutex::new(Some(stdin)));
        let own_session_id = Arc::new(Mutex::new(None));
        let closing = Arc::new(AtomicBool::new(false));

        let reader_task = spawn_reader_task(
            &executor,
            stdout,
            pending.clone(),
            stdin_holder.clone(),
            own_session_id,
            events_tx.clone(),
            closing.clone(),
        );
        let heartbeat_task = spawn_heartbeat_task(
            &executor,
            stdin_holder.clone(),
            pending.clone(),
            closing.clone(),
            events_tx,
            config.heartbeat_interval,
        );

        Ok(Self {
            socket_path,
            host: host.host.clone(),
            child: Arc::new(Mutex::new(Some(child))),
            stdin: stdin_holder,
            pending,
            closing,
            _reader_task: reader_task,
            _heartbeat_task: heartbeat_task,
        })
    }

    pub async fn send_command(&self, cmd: String) -> Result<Vec<String>, HostError> {
        let (tx, rx) = oneshot::channel();
        let line = format!("{cmd}\n");
        {
            let mut pending = self.pending.lock().await;
            let mut guard = self.stdin.lock().await;
            let stdin = guard
                .as_mut()
                .ok_or_else(|| HostError::Other("stdin closed".into()))?;
            pending.push_back(Pending::UserCommand(tx));
            if let Err(e) = stdin.write_all(line.as_bytes()).await {
                pending.pop_back();
                return Err(HostError::Other(e.to_string()));
            }
        }
        match rx.with_timeout(RESPONSE_TIMEOUT).await {
            Ok(Ok(Ok(output))) => Ok(output),
            Ok(Ok(Err(msg))) => Err(HostError::Other(msg)),
            Ok(Err(_)) | Err(_) => Err(HostError::Other("command response lost".into())),
        }
    }

    pub async fn close(&self) {
        self.closing.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.lock().await.take() {
            let _ = child.kill();
        }
        self.stdin.lock().await.take();
        stop_master(&self.host, &self.socket_path).await;
    }
}

async fn stop_master(host: &str, socket_path: &Path) {
    if !socket_path.exists() {
        return;
    }
    let mut exit = Command::new("ssh");
    exit.arg("-O")
        .arg("exit")
        .arg("-o")
        .arg(format!("ControlPath={}", socket_path.display()))
        .arg(host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let _ = exit.status().await;
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
}

fn spawn_reader_task(
    executor: &Background,
    stdout: ChildStdout,
    pending: PendingQueue,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    own_session_id: Arc<Mutex<Option<String>>>,
    events_tx: mpsc::UnboundedSender<ConnectionEvent>,
    closing: Arc<AtomicBool>,
) -> BackgroundTask {
    executor.spawn(async move {
        let mut stream = CcStream::new(stdout);
        let mut handshake_done = false;
        let mut clean_exit = false;
        while let Some(evt) = stream.next_event().await {
            match evt {
                ControlEvent::Begin { .. } => {}
                ControlEvent::End { output, .. } => {
                    let popped = pending.lock().await.pop_front();
                    match popped {
                        Some(Pending::UserCommand(tx)) => {
                            let _ = tx.send(Ok(output));
                        }
                        Some(Pending::Refresh) => {
                            emit_sessions_from(&output, &own_session_id, &events_tx).await;
                        }
                        None => {
                            if !handshake_done {
                                handshake_done = true;
                                trigger_refresh(&stdin, &pending).await;
                            }
                        }
                    }
                }
                ControlEvent::Error { output, .. } => {
                    let popped = pending.lock().await.pop_front();
                    let joined = output.join("\n");
                    match popped {
                        Some(Pending::UserCommand(tx)) => {
                            let _ = tx.send(Err(joined));
                        }
                        Some(Pending::Refresh) => {
                            if joined.contains("no server running") || joined.is_empty() {
                                emit_empty_sessions(&events_tx).await;
                            } else {
                                log::warn!("refresh failed: {joined}");
                                emit_empty_sessions(&events_tx).await;
                            }
                        }
                        None => {
                            log::warn!("unexpected tmux error with no pending: {joined}");
                        }
                    }
                }
                ControlEvent::SessionChanged { session_id, .. } => {
                    let mut g = own_session_id.lock().await;
                    if g.is_none() {
                        *g = Some(session_id);
                    }
                }
                ControlEvent::SessionsChanged
                | ControlEvent::ClientSessionChanged { .. }
                | ControlEvent::ClientDetached { .. }
                | ControlEvent::SessionRenamed { .. }
                | ControlEvent::SessionWindowChanged { .. } => {
                    trigger_refresh(&stdin, &pending).await;
                }
                ControlEvent::Exit { reason } => {
                    let mut tx = events_tx.clone();
                    let _ = tx.send(ConnectionEvent::Exit(reason)).await;
                    clean_exit = true;
                    break;
                }
                ControlEvent::ConfigError { line } => {
                    log::warn!("tmux config error: {line}");
                }
                ControlEvent::Unknown(line) => {
                    log::trace!("unknown tmux CC line: {line}");
                }
            }
        }
        if !clean_exit && !closing.load(Ordering::Relaxed) {
            signal_master_died(&stdin, &events_tx).await;
        }
    })
}

fn spawn_heartbeat_task(
    executor: &Background,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    pending: PendingQueue,
    closing: Arc<AtomicBool>,
    events_tx: mpsc::UnboundedSender<ConnectionEvent>,
    interval: Duration,
) -> BackgroundTask {
    executor.spawn(async move {
        loop {
            Timer::after(interval).await;
            if closing.load(Ordering::Relaxed) {
                break;
            }
            let (tx, rx) = oneshot::channel();
            let line = format!("{}\n", heartbeat_cmd());
            let write_result = {
                let mut queue = pending.lock().await;
                let mut guard = stdin.lock().await;
                match guard.as_mut() {
                    Some(s) => {
                        queue.push_back(Pending::UserCommand(tx));
                        s.write_all(line.as_bytes()).await
                    }
                    None => break,
                }
            };
            if let Err(e) = write_result {
                log::warn!("remote_sessions heartbeat write failed: {e}");
                signal_master_died(&stdin, &events_tx).await;
                break;
            }
            match rx.with_timeout(HEARTBEAT_TIMEOUT).await {
                Ok(Ok(Ok(_))) => {}
                other => {
                    if closing.load(Ordering::Relaxed) {
                        break;
                    }
                    log::warn!("remote_sessions heartbeat lost response: {other:?}");
                    signal_master_died(&stdin, &events_tx).await;
                    break;
                }
            }
        }
    })
}

async fn signal_master_died(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    events_tx: &mpsc::UnboundedSender<ConnectionEvent>,
) {
    let mut guard = stdin.lock().await;
    if guard.is_none() {
        return;
    }
    guard.take();
    drop(guard);
    let mut tx = events_tx.clone();
    let _ = tx.send(ConnectionEvent::Error(HostError::MasterDied)).await;
}

async fn trigger_refresh(stdin: &Arc<Mutex<Option<ChildStdin>>>, pending: &PendingQueue) {
    let line = format!("{}\n", list_sessions_cmd());
    let mut queue = pending.lock().await;
    let mut guard = stdin.lock().await;
    if let Some(s) = guard.as_mut() {
        queue.push_back(Pending::Refresh);
        if s.write_all(line.as_bytes()).await.is_err() {
            queue.pop_back();
        }
    }
}

async fn emit_sessions_from(
    output: &[String],
    own_session_id: &Arc<Mutex<Option<String>>>,
    events_tx: &mpsc::UnboundedSender<ConnectionEvent>,
) {
    let own = own_session_id.lock().await.clone();
    let sessions = parse_sessions(output, own.as_deref());
    let mut tx = events_tx.clone();
    let _ = tx.send(ConnectionEvent::Sessions(sessions)).await;
}

async fn emit_empty_sessions(events_tx: &mpsc::UnboundedSender<ConnectionEvent>) {
    let mut tx = events_tx.clone();
    let _ = tx.send(ConnectionEvent::Sessions(Vec::new())).await;
}

async fn ensure_master(host: &RemoteHost, socket_path: &Path) -> Result<(), HostError> {
    if check_master(host, socket_path).await {
        return Ok(());
    }
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    let mut master = Command::new("ssh");
    master
        .arg("-N")
        .arg("-o")
        .arg("ControlMaster=yes")
        .arg("-o")
        .arg(format!("ControlPath={}", socket_path.display()))
        .arg("-o")
        .arg("ControlPersist=60")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .args(target_args(host))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(false);

    let mut master_child = master
        .spawn()
        .map_err(|e| HostError::Other(e.to_string()))?;

    for _ in 0..MASTER_PROBE_ATTEMPTS {
        Timer::after(MASTER_PROBE_DELAY).await;
        if check_master(host, socket_path).await {
            return Ok(());
        }
        if let Ok(Some(_)) = master_child.try_status() {
            let detail = drain_stderr(&mut master_child).await;
            return Err(classify_ssh_error(&detail));
        }
    }
    let detail = drain_stderr(&mut master_child).await;
    let _ = master_child.kill();
    if detail.is_empty() {
        Err(HostError::HostUnreachable(
            "ControlMaster handshake timed out".into(),
        ))
    } else {
        Err(classify_ssh_error(&detail))
    }
}

async fn drain_stderr(child: &mut Child) -> String {
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        String::new()
    }
}

async fn check_master(host: &RemoteHost, socket_path: &Path) -> bool {
    let mut check = Command::new("ssh");
    check
        .arg("-O")
        .arg("check")
        .arg("-o")
        .arg(format!("ControlPath={}", socket_path.display()))
        .arg(&host.host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    matches!(check.status().await, Ok(s) if s.success())
}

fn ensure_socket_dir(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
