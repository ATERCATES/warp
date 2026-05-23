use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use futures::StreamExt;
use settings::Setting as _;
use warpui::r#async::executor::{Background, BackgroundTask};
use warpui::{AppContext, Entity, ModelContext, ModelHandle, ModelSpawner, SingletonEntity};

use crate::settings::remote_hosts::{RemoteHost, RemoteSessionsSettings};
use crate::terminal::remote_sessions::commands::{kill_session_cmd, new_session_cmd};
use crate::terminal::remote_sessions::connection::{
    ConnectionConfig, ConnectionEvent, RemoteHostConnection,
};
use crate::terminal::remote_sessions::probe::probe_host;
use crate::terminal::remote_sessions::types::{HostError, HostState, HostStatus};

#[derive(Clone, Debug)]
pub enum RemoteSessionsEvent {
    HostAdded(String),
    HostRemoved(String),
    HostStatusChanged(String),
    HostCapabilitiesUpdated(String),
    HostSessionsUpdated(String),
}

pub struct RemoteSessionsModel {
    hosts: HashMap<String, HostState>,
    connections: HashMap<String, Arc<RemoteHostConnection>>,
    event_tasks: HashMap<String, BackgroundTask>,
}

impl Entity for RemoteSessionsModel {
    type Event = RemoteSessionsEvent;
}

impl SingletonEntity for RemoteSessionsModel {}

impl RemoteSessionsModel {
    pub fn register(ctx: &mut AppContext) {
        let handle: ModelHandle<Self> = ctx.add_singleton_model(|_| Self::new());
        let _ = handle;
    }

    fn new() -> Self {
        sweep_stale_sockets();
        Self {
            hosts: HashMap::new(),
            connections: HashMap::new(),
            event_tasks: HashMap::new(),
        }
    }

    pub fn ingest_hosts(&mut self, hosts: &[RemoteHost], ctx: &mut ModelContext<Self>) {
        let keys_now: Vec<String> = hosts.iter().map(|h| h.local_host_key.clone()).collect();
        for host in hosts {
            if !self.hosts.contains_key(&host.local_host_key) {
                self.hosts.insert(
                    host.local_host_key.clone(),
                    HostState {
                        local_host_key: host.local_host_key.clone(),
                        status: HostStatus::Disconnected,
                        capabilities: None,
                        sessions: Vec::new(),
                        last_error_detail: None,
                    },
                );
                ctx.emit(RemoteSessionsEvent::HostAdded(host.local_host_key.clone()));
            }
        }
        let removed: Vec<String> = self
            .hosts
            .keys()
            .filter(|k| !keys_now.contains(k))
            .cloned()
            .collect();
        for key in removed {
            self.hosts.remove(&key);
            self.connections.remove(&key);
            self.event_tasks.remove(&key);
            ctx.emit(RemoteSessionsEvent::HostRemoved(key));
        }
    }

    pub fn host_state(&self, key: &str) -> Option<&HostState> {
        self.hosts.get(key)
    }

    pub fn connect(
        &mut self,
        host: RemoteHost,
        settings: &RemoteSessionsSettings,
        ctx: &mut ModelContext<Self>,
    ) {
        let key = host.local_host_key.clone();
        if self.connections.contains_key(&key)
            || matches!(
                self.hosts.get(&key).map(|s| &s.status),
                Some(HostStatus::Connecting)
            )
        {
            return;
        }
        self.set_status(&key, HostStatus::Connecting, ctx);
        let executor = ctx.background_executor();
        let host_for_task = host.clone();
        let socket = socket_path_for(&key);
        let config = ConnectionConfig {
            heartbeat_interval: Duration::from_secs(
                (*settings.heartbeat_interval_seconds.value()) as u64,
            ),
        };

        ctx.spawn(
            async move {
                let caps = probe_host(&host_for_task).await?;
                if !caps.tmux_supported {
                    return Err(HostError::Other(format!(
                        "tmux {} not supported (need 3.2+)",
                        caps.tmux_version
                    )));
                }
                let (tx, rx) = mpsc::unbounded::<ConnectionEvent>();
                let conn = RemoteHostConnection::open(
                    &host_for_task,
                    socket,
                    tx,
                    executor.clone(),
                    config,
                )
                .await?;
                Ok::<_, HostError>((caps, conn, rx))
            },
            move |model, result, ctx| match result {
                Ok((caps, conn, rx)) => {
                    if let Some(state) = model.hosts.get_mut(&key) {
                        state.capabilities = Some(caps);
                        state.last_error_detail = None;
                    }
                    ctx.emit(RemoteSessionsEvent::HostCapabilitiesUpdated(key.clone()));
                    model.connections.insert(key.clone(), Arc::new(conn));
                    model.set_status(&key, HostStatus::Connected, ctx);
                    model.spawn_event_consumer(key.clone(), rx, ctx);
                }
                Err(e) => {
                    let detail = e.to_string();
                    if let Some(state) = model.hosts.get_mut(&key) {
                        state.last_error_detail = Some(detail.clone());
                    }
                    let status = match &e {
                        HostError::Other(s) if s.to_ascii_lowercase().contains("tmux") => {
                            HostStatus::Unsupported(s.clone())
                        }
                        _ => HostStatus::Error(e),
                    };
                    model.set_status(&key, status, ctx);
                }
            },
        );
    }

    pub fn disconnect(&mut self, key: &str, ctx: &mut ModelContext<Self>) {
        if let Some(conn) = self.connections.remove(key) {
            ctx.spawn(
                async move {
                    conn.close().await;
                },
                |_, _, _| {},
            );
        }
        self.event_tasks.remove(key);
        if let Some(state) = self.hosts.get_mut(key) {
            state.sessions.clear();
        }
        self.set_status(key, HostStatus::Disconnected, ctx);
    }

    pub fn create_session(
        &mut self,
        key: &str,
        name: String,
        command: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.run_session_cmd(key, new_session_cmd(&name, command.as_deref()), "create_session", ctx);
    }

    pub fn kill_session(&mut self, key: &str, name: String, ctx: &mut ModelContext<Self>) {
        self.run_session_cmd(key, kill_session_cmd(&name), "kill_session", ctx);
    }

    fn run_session_cmd(
        &mut self,
        key: &str,
        cmd: String,
        label: &'static str,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(conn) = self.connections.get(key).cloned() else {
            return;
        };
        ctx.spawn(
            async move { conn.send_command(cmd).await },
            move |_, result, _| {
                if let Err(e) = result {
                    log::warn!("{label} failed: {e:?}");
                }
            },
        );
    }

    fn spawn_event_consumer(
        &mut self,
        key: String,
        mut rx: mpsc::UnboundedReceiver<ConnectionEvent>,
        ctx: &mut ModelContext<Self>,
    ) {
        let spawner: ModelSpawner<Self> = ctx.spawner();
        let executor: Arc<Background> = ctx.background_executor();
        let key_for_task = key.clone();
        let task = executor.spawn(async move {
            while let Some(evt) = rx.next().await {
                let k = key_for_task.clone();
                let _ = spawner
                    .spawn(move |me, ctx| me.handle_connection_event(&k, evt, ctx))
                    .await;
            }
        });
        self.event_tasks.insert(key, task);
    }

    fn handle_connection_event(
        &mut self,
        key: &str,
        event: ConnectionEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            ConnectionEvent::Sessions(sessions) => {
                if let Some(state) = self.hosts.get_mut(key) {
                    state.sessions = sessions;
                }
                ctx.emit(RemoteSessionsEvent::HostSessionsUpdated(key.to_string()));
            }
            ConnectionEvent::Exit(reason) => {
                self.connections.remove(key);
                if let Some(state) = self.hosts.get_mut(key) {
                    state.sessions.clear();
                    state.last_error_detail = reason;
                }
                self.set_status(key, HostStatus::Disconnected, ctx);
            }
            ConnectionEvent::Error(err) => {
                self.connections.remove(key);
                let detail = err.to_string();
                if let Some(state) = self.hosts.get_mut(key) {
                    state.sessions.clear();
                    state.last_error_detail = Some(detail);
                }
                self.set_status(key, HostStatus::Error(err), ctx);
            }
        }
    }

    fn set_status(&mut self, key: &str, status: HostStatus, ctx: &mut ModelContext<Self>) {
        if let Some(state) = self.hosts.get_mut(key) {
            state.status = status;
        }
        ctx.emit(RemoteSessionsEvent::HostStatusChanged(key.to_string()));
    }
}

pub fn socket_path_for(key: &str) -> PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(key, &mut hasher);
    let short = format!("{:016x}", std::hash::Hasher::finish(&hasher));
    let mut path = std::env::temp_dir();
    path.push("warp-rs");
    path.push(format!("{short}.sock"));
    path
}

fn sweep_stale_sockets() {
    let mut dir = std::env::temp_dir();
    dir.push("warp-rs");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("sock") {
            let _ = std::fs::remove_file(&path);
        }
    }
}
