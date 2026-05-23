use async_process::ChildStdout;
use futures::io::{AsyncBufReadExt, AsyncRead, BufReader};

#[derive(Debug, Clone, PartialEq)]
pub enum ControlEvent {
    Begin {
        id: u32,
    },
    End {
        id: u32,
        output: Vec<String>,
    },
    Error {
        id: u32,
        output: Vec<String>,
    },
    SessionsChanged,
    SessionChanged {
        session_id: String,
        name: String,
    },
    ClientSessionChanged {
        tty: String,
        session_id: String,
        name: String,
    },
    ClientDetached {
        tty: String,
    },
    SessionRenamed {
        session_id: String,
        new_name: String,
    },
    SessionWindowChanged {
        session_id: String,
    },
    Exit {
        reason: Option<String>,
    },
    ConfigError {
        line: String,
    },
    Unknown(String),
}

pub struct CcStream<R = ChildStdout> {
    reader: BufReader<R>,
    pending_block: Option<(u32, bool, Vec<String>)>,
}

impl<R: AsyncRead + Unpin> CcStream<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            pending_block: None,
        }
    }

    pub async fn next_event(&mut self) -> Option<ControlEvent> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.reader.read_line(&mut line).await.ok()?;
            if n == 0 {
                return None;
            }
            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
            if let Some(rest) = trimmed.strip_prefix("%begin ") {
                let id = parse_block_header_id(rest)?;
                self.pending_block = Some((id, false, Vec::new()));
                return Some(ControlEvent::Begin { id });
            }
            if let Some(rest) = trimmed.strip_prefix("%end ") {
                let id = parse_block_header_id(rest)?;
                let output = self
                    .pending_block
                    .take()
                    .map(|(_, _, o)| o)
                    .unwrap_or_default();
                return Some(ControlEvent::End { id, output });
            }
            if let Some(rest) = trimmed.strip_prefix("%error ") {
                let id = parse_block_header_id(rest)?;
                let output = self
                    .pending_block
                    .take()
                    .map(|(_, _, o)| o)
                    .unwrap_or_default();
                return Some(ControlEvent::Error { id, output });
            }
            if let Some(block) = self.pending_block.as_mut() {
                block.2.push(trimmed.to_string());
                continue;
            }
            if trimmed == "%sessions-changed" {
                return Some(ControlEvent::SessionsChanged);
            }
            if let Some(rest) = trimmed.strip_prefix("%session-changed ") {
                let mut parts = rest.splitn(2, ' ');
                let id = parts.next()?.to_string();
                let name = parts.next().unwrap_or("").to_string();
                return Some(ControlEvent::SessionChanged {
                    session_id: id,
                    name,
                });
            }
            if let Some(rest) = trimmed.strip_prefix("%client-session-changed ") {
                let mut parts = rest.splitn(3, ' ');
                let tty = parts.next()?.to_string();
                let sid = parts.next()?.to_string();
                let name = parts.next().unwrap_or("").to_string();
                return Some(ControlEvent::ClientSessionChanged {
                    tty,
                    session_id: sid,
                    name,
                });
            }
            if let Some(rest) = trimmed.strip_prefix("%client-detached ") {
                return Some(ControlEvent::ClientDetached {
                    tty: rest.to_string(),
                });
            }
            if let Some(rest) = trimmed.strip_prefix("%session-renamed ") {
                let mut parts = rest.splitn(2, ' ');
                let id = parts.next()?.to_string();
                let name = parts.next().unwrap_or("").to_string();
                return Some(ControlEvent::SessionRenamed {
                    session_id: id,
                    new_name: name,
                });
            }
            if let Some(rest) = trimmed.strip_prefix("%session-window-changed ") {
                return Some(ControlEvent::SessionWindowChanged {
                    session_id: rest.to_string(),
                });
            }
            if let Some(rest) = trimmed.strip_prefix("%exit") {
                let reason = rest.trim().to_string();
                return Some(ControlEvent::Exit {
                    reason: if reason.is_empty() {
                        None
                    } else {
                        Some(reason)
                    },
                });
            }
            if let Some(rest) = trimmed.strip_prefix("%config-error ") {
                return Some(ControlEvent::ConfigError {
                    line: rest.to_string(),
                });
            }
            if trimmed.starts_with('%') {
                return Some(ControlEvent::Unknown(trimmed.to_string()));
            }
        }
    }
}

fn parse_block_header_id(s: &str) -> Option<u32> {
    let mut it = s.split_whitespace();
    let _time = it.next()?;
    let id = it.next()?.parse().ok()?;
    Some(id)
}

#[cfg(test)]
#[path = "cc_parser_tests.rs"]
mod tests;
