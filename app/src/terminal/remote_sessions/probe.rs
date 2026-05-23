use std::process::Stdio;
use std::time::Duration;

use command::r#async::Command;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use warpui::r#async::FutureExt as _;

use crate::settings::remote_hosts::RemoteHost;
use crate::terminal::remote_sessions::ssh_args::target_args;
use crate::terminal::remote_sessions::types::{HostCapabilities, HostError};

const PROBE_SCRIPT: &str =
    include_str!("../../../assets/bundled/ssh/bash_zsh/probe_remote_host.sh");
const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const BEGIN_MARKER: &str = "__WARP_REMOTE_SESSIONS_PROBE_BEGIN__";
const END_MARKER: &str = "__WARP_REMOTE_SESSIONS_PROBE_END__";

pub async fn probe_host(host: &RemoteHost) -> Result<HostCapabilities, HostError> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .args(target_args(host))
        .arg("bash -s")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| HostError::Other(e.to_string()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(PROBE_SCRIPT.as_bytes())
            .await
            .map_err(|e| HostError::Other(e.to_string()))?;
        drop(stdin);
    }

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| HostError::TmuxFailedToStart("probe stdout missing".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| HostError::TmuxFailedToStart("probe stderr missing".into()))?;
    let mut out_buf = Vec::new();
    let mut err_buf = Vec::new();

    let outcome = async {
        let (_, _, status) = futures::future::join3(
            stdout.read_to_end(&mut out_buf),
            stderr.read_to_end(&mut err_buf),
            child.status(),
        )
        .await;
        status
    }
    .with_timeout(PROBE_TIMEOUT)
    .await
    .map_err(|_| HostError::ProbeTimedOut)?
    .map_err(|e| HostError::Other(e.to_string()))?;

    if !outcome.success() {
        let stderr_text = String::from_utf8_lossy(&err_buf);
        return Err(classify_ssh_error(&stderr_text));
    }

    let stdout_text = String::from_utf8_lossy(&out_buf);
    extract_capabilities(&stdout_text)
}

fn extract_capabilities(out: &str) -> Result<HostCapabilities, HostError> {
    let begin = out
        .find(BEGIN_MARKER)
        .ok_or_else(|| HostError::ProbeMalformed("no begin marker".into()))?
        + BEGIN_MARKER.len();
    let rest = &out[begin..];
    let end = rest
        .find(END_MARKER)
        .ok_or_else(|| HostError::ProbeMalformed("no end marker".into()))?;
    let json = rest[..end].trim();
    serde_json::from_str::<HostCapabilities>(json)
        .map_err(|e| HostError::ProbeMalformed(format!("{e}: {json}")))
}

pub(super) fn classify_ssh_error(stderr_text: &str) -> HostError {
    let lower = stderr_text.to_ascii_lowercase();
    let detail = stderr_text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string();
    if lower.contains("permission denied") || lower.contains("authentication") {
        HostError::SshAuthFailed(detail)
    } else if lower.contains("could not resolve")
        || lower.contains("no route to host")
        || lower.contains("connection refused")
        || lower.contains("connection timed out")
        || lower.contains("operation timed out")
        || lower.contains("network is unreachable")
    {
        HostError::HostUnreachable(detail)
    } else if detail.is_empty() {
        HostError::Other("ssh failed".into())
    } else {
        HostError::Other(detail)
    }
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
