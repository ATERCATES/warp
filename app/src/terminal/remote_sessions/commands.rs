use crate::terminal::remote_sessions::types::RemoteTmuxSession;

pub const SESSIONS_FORMAT: &str =
    "#{session_id}|#{session_name}|#{session_created}|#{session_attached}|#{pane_pid}|#{pane_current_command}";

pub const CONTROL_SESSION_NAME: &str = "__warp_ctrl";

pub fn list_sessions_cmd() -> String {
    format!("list-sessions -F '{}'", SESSIONS_FORMAT)
}

pub fn new_session_cmd(name: &str, command: Option<&str>) -> String {
    match command {
        Some(c) if !c.is_empty() => format!(
            "new-session -d -s {} {}",
            shell_escape(name),
            shell_escape(c)
        ),
        _ => format!("new-session -d -s {}", shell_escape(name)),
    }
}

pub fn kill_session_cmd(name: &str) -> String {
    format!("kill-session -t {}", shell_escape(name))
}

pub fn heartbeat_cmd() -> &'static str {
    "display-message -p '#{client_pid}'"
}

pub fn parse_sessions(lines: &[String], exclude_session_id: Option<&str>) -> Vec<RemoteTmuxSession> {
    lines
        .iter()
        .filter_map(|l| {
            let mut parts = l.split('|');
            let sid = parts.next()?.to_string();
            if Some(sid.as_str()) == exclude_session_id {
                return None;
            }
            let name = parts.next()?.to_string();
            if name == CONTROL_SESSION_NAME {
                return None;
            }
            let created = parts.next()?.parse().ok()?;
            let attached = parts.next()?.parse().ok()?;
            let _pid = parts.next();
            let cur = parts.next().unwrap_or("").to_string();
            Some(RemoteTmuxSession {
                session_id: sid,
                name,
                created_unix: created,
                attached_count: attached,
                current_command: cur,
            })
        })
        .collect()
}

fn shell_escape(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        s.to_string()
    } else {
        let mut escaped = String::from("'");
        for ch in s.chars() {
            if ch == '\'' {
                escaped.push_str("'\\''");
            } else {
                escaped.push(ch);
            }
        }
        escaped.push('\'');
        escaped
    }
}
