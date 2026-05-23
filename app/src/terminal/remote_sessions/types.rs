use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub tmux_bin: String,
    pub tmux_version: String,
    pub tmux_supported: bool,
    pub passthrough: bool,
    pub shell_integration: bool,
    pub os: String,
    pub pkg: String,
    pub root_access: RootAccess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootAccess {
    IsRoot,
    CanRunSudo,
    NoRootAccess,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostStatus {
    Disconnected,
    Connecting,
    Connected,
    Error(HostError),
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum HostError {
    SshAuthFailed(String),
    HostUnreachable(String),
    ProbeTimedOut,
    ProbeMalformed(String),
    TmuxFailedToStart(String),
    MasterDied,
    Other(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::SshAuthFailed(detail) if !detail.is_empty() => {
                write!(f, "SSH authentication failed: {detail}")
            }
            HostError::SshAuthFailed(_) => write!(f, "SSH authentication failed"),
            HostError::HostUnreachable(detail) if !detail.is_empty() => {
                write!(f, "Host unreachable: {detail}")
            }
            HostError::HostUnreachable(_) => write!(f, "Host unreachable"),
            HostError::ProbeTimedOut => write!(f, "Probe timed out"),
            HostError::ProbeMalformed(detail) => write!(f, "Probe response malformed: {detail}"),
            HostError::TmuxFailedToStart(detail) if !detail.is_empty() => {
                write!(f, "tmux failed to start: {detail}")
            }
            HostError::TmuxFailedToStart(_) => write!(f, "tmux failed to start"),
            HostError::MasterDied => write!(f, "Control connection died"),
            HostError::Other(detail) => write!(f, "{detail}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteTmuxSession {
    pub session_id: String,
    pub name: String,
    pub created_unix: i64,
    pub attached_count: u32,
    pub current_command: String,
}

#[derive(Clone, Debug)]
pub struct HostState {
    pub local_host_key: String,
    pub status: HostStatus,
    pub capabilities: Option<HostCapabilities>,
    pub sessions: Vec<RemoteTmuxSession>,
    pub last_error_detail: Option<String>,
}
