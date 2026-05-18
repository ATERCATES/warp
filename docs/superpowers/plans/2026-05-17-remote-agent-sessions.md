# Persistent Remote CLI Agent Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add persistent remote CLI agent sessions to Warp: agents (Claude Code, Codex, Gemini CLI, OpenCode) running on a user's SSH host survive client disconnect and can be re-attached from another machine with kick semantics.

**Architecture:** Extend the existing `remote_server` daemon (`crates/remote_server/`) with a new `AgentSessionManager` module holding per-session `(child, pty, ring_buffer, attach_lock)`. Add 9 new protobuf messages travelling through the existing SSH transport. Client-side: a new pane type, sessions list view, resolver/registry mirroring PR #11097/#10426/#10510 patterns, heartbeat task, and external-editor URL helper. All gated behind `FeatureFlag::RemoteAgentSessions`.

**Tech Stack:** Rust (edition 2021), Tokio async, `portable-pty`, `prost` proto3, Diesel (untouched), `warpui` framework, `cargo nextest` for tests. Spec lives at `specs/gh-9416/{PRODUCT,TECH,OBJECTIVE}.md`.

**Spec invariants:** All task references like `(P-N)` map to behavior numbers in `specs/gh-9416/PRODUCT.md`.

---

## Phase 0 — Prelude: protocol schema and feature flag

Prelude must land before any other phase. Creates the proto vocabulary and the feature flag both ends will gate on.

### Task 0.1: Add feature flag variants

**Files:**
- Modify: `crates/warp_features/src/lib.rs`

- [ ] **Step 1: Add the variants to `FeatureFlag` enum**

In `crates/warp_features/src/lib.rs`, locate `pub enum FeatureFlag` (around line 9) and add two new variants alphabetically near the existing `Remote*` flags:

```rust
RemoteAgentSessions,
RemoteAgentSessionsDebug,
```

- [ ] **Step 2: Add to `DOGFOOD_FLAGS`**

Locate `pub const DOGFOOD_FLAGS: &[FeatureFlag] = &[` (around line 901) and add:

```rust
FeatureFlag::RemoteAgentSessions,
```

Do NOT add `RemoteAgentSessionsDebug` to dogfood — it stays off by default even for the team.

- [ ] **Step 3: Verify build**

Run: `cargo check -p warp_features`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
git add crates/warp_features/src/lib.rs
git commit -m "Add RemoteAgentSessions feature flags"
```

### Task 0.2: Add protobuf message definitions

**Files:**
- Modify: `crates/remote_server/proto/remote_server.proto`

- [ ] **Step 1: Add `AgentKind` enum and request messages**

At the end of `crates/remote_server/proto/remote_server.proto` add:

```proto
// ── Remote agent sessions (gh-9416) ──────────────────────────────────────────

enum AgentKind {
  AGENT_KIND_UNSPECIFIED = 0;
  AGENT_KIND_CLAUDE = 1;
  AGENT_KIND_CODEX = 2;
  AGENT_KIND_GEMINI = 3;
  AGENT_KIND_OPENCODE = 4;
  AGENT_KIND_CUSTOM = 5;
}

message StartAgentSessionRequest {
  optional string requested_label = 1;
  string cwd = 2;
  AgentKind agent_kind = 3;
  optional string custom_command = 4;
  repeated string custom_args = 5;
}

message ListAgentSessionsRequest {}

message AttachAgentSessionRequest {
  string session_id = 1;
  optional uint64 from_offset = 2;
  uint32 cols = 3;
  uint32 rows = 4;
}

message HeartbeatAgentSessionRequest { string session_id = 1; }
message DetachAgentSessionRequest { string session_id = 1; }
message KillAgentSessionRequest { string session_id = 1; }
message RenameAgentSessionRequest { string session_id = 1; string new_label = 2; }
message AgentSessionInputRequest { string session_id = 1; bytes bytes = 2; }
message AgentSessionResizeRequest { string session_id = 1; uint32 cols = 2; uint32 rows = 3; }
message InspectAgentSessionRequest { string session_id = 1; }
```

- [ ] **Step 2: Add response messages**

Continue in the same file:

```proto
message StartedSession {
  string session_id = 1;
  string label = 2;
  int64 started_at_unix_ms = 3;
}

message StartError {
  enum Kind {
    KIND_UNSPECIFIED = 0;
    KIND_COMMAND_NOT_FOUND = 1;
    KIND_CWD_INVALID = 2;
    KIND_OS_ERROR = 3;
    KIND_UNSUPPORTED = 4;
  }
  Kind kind = 1;
  string detail = 2;
}

message StartAgentSessionResponse {
  oneof result {
    StartedSession started = 1;
    StartError error = 2;
  }
}

message RunningState { uint32 pid = 1; }
message EndedState {
  int64 ended_at_unix_ms = 1;
  oneof exit { int32 code = 2; int32 signal = 3; }
}

message SessionSummary {
  string session_id = 1;
  string label = 2;
  AgentKind agent_kind = 3;
  string cwd = 4;
  int64 started_at_unix_ms = 5;
  int64 last_active_unix_ms = 6;
  oneof state { RunningState running = 7; EndedState ended = 8; }
  optional string attached_client_label = 9;
}

message ListAgentSessionsResponse { repeated SessionSummary sessions = 1; }

message AttachSnapshot {
  bytes buffer = 1;
  bool truncated_from_start = 2;
  uint64 current_offset = 3;
}
message AttachLive { bytes chunk = 1; uint64 offset = 2; }
message AttachDetached {
  enum Reason {
    REASON_UNSPECIFIED = 0;
    REASON_CLIENT_REQUESTED = 1;
    REASON_SUPERSEDED = 2;
    REASON_HEARTBEAT_TIMEOUT = 3;
    REASON_DAEMON_SHUTTING_DOWN = 4;
  }
  Reason reason = 1;
  optional string superseding_client_label = 2;
}
message AttachSessionEnded { EndedState ended = 1; }
message AttachError {
  enum Kind {
    KIND_UNSPECIFIED = 0;
    KIND_NOT_FOUND = 1;
    KIND_SESSION_ENDED_NO_BUFFER = 2;
  }
  Kind kind = 1;
}

message AttachAgentSessionEvent {
  oneof event {
    AttachSnapshot snapshot = 1;
    AttachLive live = 2;
    AttachDetached detached = 3;
    AttachSessionEnded session_ended = 4;
    AttachError error = 5;
  }
}

message AttachHistoryEntry {
  string client_label = 1;
  int64 attached_at_unix_ms = 2;
  optional int64 detached_at_unix_ms = 3;
  optional AttachDetached.Reason detach_reason = 4;
}

message ChildStatusDescriptor {
  oneof status { RunningState running = 1; EndedState ended = 2; }
}

message InspectAgentSessionResponse {
  uint64 ring_buffer_bytes = 1;
  uint64 ring_buffer_truncations = 2;
  repeated AttachHistoryEntry attach_history = 3;
  optional int64 last_heartbeat_unix_ms = 4;
  ChildStatusDescriptor child_status = 5;
}

message AgentSessionGenericResponse {
  oneof result { Empty ok = 1; string error = 2; }
}
message Empty {}
```

- [ ] **Step 3: Wire the new requests into `ClientMessage` oneof**

Locate the `oneof message` block inside `message ClientMessage` (around line 17). Append (use the next free field numbers — inspect the existing block to find them; assume the file currently uses up to 12, start at 13):

```proto
StartAgentSessionRequest start_agent_session = 13;
ListAgentSessionsRequest list_agent_sessions = 14;
AttachAgentSessionRequest attach_agent_session = 15;
HeartbeatAgentSessionRequest heartbeat_agent_session = 16;
DetachAgentSessionRequest detach_agent_session = 17;
KillAgentSessionRequest kill_agent_session = 18;
RenameAgentSessionRequest rename_agent_session = 19;
AgentSessionInputRequest agent_session_input = 20;
AgentSessionResizeRequest agent_session_resize = 21;
InspectAgentSessionRequest inspect_agent_session = 22;
```

(If the file already uses fields beyond 12, shift the numbers accordingly. Always read the current state first.)

- [ ] **Step 4: Wire the new responses and push event into `ServerMessage` oneof**

Inside `message ServerMessage` `oneof message` block, append using the next free field numbers:

```proto
StartAgentSessionResponse start_agent_session_response = N;
ListAgentSessionsResponse list_agent_sessions_response = N+1;
AttachAgentSessionEvent attach_agent_session_event = N+2;
InspectAgentSessionResponse inspect_agent_session_response = N+3;
AgentSessionGenericResponse agent_session_generic_response = N+4;
```

(Replace N with actual next free number from the file.)

- [ ] **Step 5: Add `agent_sessions_v1` field to `SessionBootstrapped`**

Locate `message SessionBootstrapped` (line ~122). Add a new oneof or a bool field. Use a bool for simplicity:

```proto
message SessionBootstrapped {
  uint64 session_id = 1;
  string shell_type = 2;
  optional string shell_path = 3;
  // Daemon-side capability flag. True if this daemon supports agent_sessions_v1.
  // Older daemons leave this absent/false; newer clients gate UI on this.
  bool agent_sessions_v1 = 4;
}
```

- [ ] **Step 6: Build to regenerate prost types**

Run: `cargo build -p remote_server`
Expected: builds clean. The `build.rs` regenerates Rust types from the proto.

- [ ] **Step 7: Commit**

```bash
git add crates/remote_server/proto/remote_server.proto
git commit -m "Add agent sessions proto schema"
```

### Task 0.3: Advertise `agent_sessions_v1` at daemon bootstrap

**Files:**
- Modify: `app/src/remote_server/server_model.rs` (location where `SessionBootstrapped` is constructed on the daemon side; verify with `grep -n SessionBootstrapped app/src/remote_server/server_model.rs`)

- [ ] **Step 1: Find where the daemon emits `SessionBootstrapped`**

Run: `grep -rn "SessionBootstrapped {" app/src/remote_server/ crates/remote_server/src/`
Note the file and line. This is the construction site.

- [ ] **Step 2: Add `agent_sessions_v1: true` to that construction**

Find the struct literal and add the field. Example shape:

```rust
SessionBootstrapped {
    session_id,
    shell_type,
    shell_path,
    agent_sessions_v1: true,
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p remote_server`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "Advertise agent_sessions_v1 at session bootstrap"
```

### Task 0.4: Surface daemon advertisement to client

**Files:**
- Modify: `crates/remote_server/src/client/mod.rs` (or wherever the client side reads `SessionBootstrapped`)

- [ ] **Step 1: Find where the client receives `SessionBootstrapped`**

Run: `grep -rn "SessionBootstrapped" crates/remote_server/src/ app/src/terminal/writeable_pty/remote_server_controller.rs`

- [ ] **Step 2: Add a getter on the per-host session state**

Wherever `RemoteSessionState` keeps post-bootstrap capabilities, store `agent_sessions_v1: bool`. Example:

```rust
// in the connected variant of RemoteSessionState
pub fn agent_sessions_v1_supported(&self) -> bool {
    self.bootstrapped_caps.agent_sessions_v1
}
```

The exact integration point depends on existing `RemoteSessionState` shape — read it before editing.

- [ ] **Step 3: Build**

Run: `cargo build -p warp`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "Surface agent_sessions_v1 capability to client"
```

### Task 0.5: Proto roundtrip tests

**Files:**
- Modify: `crates/remote_server/src/protocol_tests.rs`

- [ ] **Step 1: Write a roundtrip test for each new message**

Add to `protocol_tests.rs`:

```rust
#[test]
fn start_agent_session_request_roundtrip() {
    use crate::proto::*;
    let req = StartAgentSessionRequest {
        requested_label: Some("my session".into()),
        cwd: "/home/user".into(),
        agent_kind: AgentKind::Claude as i32,
        custom_command: None,
        custom_args: vec![],
    };
    let bytes = prost::Message::encode_to_vec(&req);
    let decoded = StartAgentSessionRequest::decode(bytes.as_slice()).unwrap();
    assert_eq!(req, decoded);
}

#[test]
fn attach_agent_session_event_oneof_roundtrip() {
    use crate::proto::*;
    let event = AttachAgentSessionEvent {
        event: Some(attach_agent_session_event::Event::Snapshot(AttachSnapshot {
            buffer: vec![1, 2, 3],
            truncated_from_start: true,
            current_offset: 42,
        })),
    };
    let bytes = prost::Message::encode_to_vec(&event);
    let decoded = AttachAgentSessionEvent::decode(bytes.as_slice()).unwrap();
    assert_eq!(event, decoded);
}
```

Repeat for: `ListAgentSessionsResponse`, `SessionSummary` with both `Running` and `Ended` states, `AttachDetached` with each `Reason`, `InspectAgentSessionResponse`, `StartError` with each `Kind`, `AgentSessionGenericResponse` with both ok and error variants.

- [ ] **Step 2: Run tests**

Run: `cargo nextest run -p remote_server protocol_tests::`
Expected: all new tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/remote_server/src/protocol_tests.rs
git commit -m "Add roundtrip tests for agent session proto messages"
```

---

## Phase 1 — Daemon: AgentSessionManager and handlers

After Prelude lands. Owns `crates/remote_server/src/agent_sessions/` and `crates/remote_server/src/server_handlers/agent_sessions.rs`. Heavy TDD.

### Task 1.1: Scaffold the module

**Files:**
- Create: `crates/remote_server/src/agent_sessions/mod.rs`
- Create: `crates/remote_server/src/agent_sessions/ring_buffer.rs`
- Create: `crates/remote_server/src/agent_sessions/session.rs`
- Create: `crates/remote_server/src/agent_sessions/manager.rs`
- Create: `crates/remote_server/src/agent_sessions/types.rs`
- Modify: `crates/remote_server/src/lib.rs`

- [ ] **Step 1: Add module declaration in lib.rs**

In `crates/remote_server/src/lib.rs`:

```rust
pub mod agent_sessions;
```

- [ ] **Step 2: Create `agent_sessions/mod.rs`**

```rust
pub mod manager;
pub mod ring_buffer;
pub mod session;
pub mod types;

pub use manager::AgentSessionManager;
pub use types::{
    AgentKind, AttachError, ClientId, DetachReason, ExitDescriptor, SessionId,
    SessionState, SessionSummary, StartError,
};

pub const MAX_ENDED_SESSIONS: usize = 20;
pub const RING_BUFFER_CAP_BYTES: usize = 4 * 1024 * 1024;
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 25;
pub const HEARTBEAT_SWEEP_INTERVAL_SECS: u64 = 5;
```

- [ ] **Step 3: Create `agent_sessions/types.rs` with shared types**

```rust
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    Claude,
    Codex,
    Gemini,
    OpenCode,
    Custom,
}

#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: SessionId,
    pub label: String,
    pub agent_kind: AgentKind,
    pub cwd: String,
    pub started_at: SystemTime,
    pub last_active: SystemTime,
    pub state: SessionStateDescriptor,
    pub attached_client_label: Option<String>,
}

#[derive(Clone, Debug)]
pub enum SessionStateDescriptor {
    Running { pid: u32 },
    Ended { ended_at: SystemTime, exit: ExitDescriptor },
}

#[derive(Clone, Copy, Debug)]
pub enum ExitDescriptor {
    Code(i32),
    Signal(i32),
}

pub enum SessionState {
    Running { pid: u32 },
    Ended { exit: ExitDescriptor, ended_at: SystemTime },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachReason {
    ClientRequested,
    Superseded,
    HeartbeatTimeout,
    DaemonShuttingDown,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("command not found: {0}")]
    CommandNotFound(String),
    #[error("cwd invalid: {0}")]
    CwdInvalid(String),
    #[error("os error: {0}")]
    OsError(String),
    #[error("unsupported")]
    Unsupported,
}

#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    #[error("session not found")]
    NotFound,
}
```

- [ ] **Step 4: Stub the other files**

Create `ring_buffer.rs`, `session.rs`, `manager.rs` each with a single `// TODO: implement` comment so the module compiles.

- [ ] **Step 5: Verify build**

Run: `cargo check -p remote_server`
Expected: builds clean.

- [ ] **Step 6: Commit**

```bash
git add crates/remote_server/src/lib.rs crates/remote_server/src/agent_sessions/
git commit -m "Scaffold agent_sessions module"
```

### Task 1.2: RingBuffer — push and snapshot, basic case

**Files:**
- Create: `crates/remote_server/src/agent_sessions/ring_buffer.rs`
- Create: `crates/remote_server/src/agent_sessions/ring_buffer_tests.rs`

- [ ] **Step 1: Write the failing test**

In `ring_buffer_tests.rs`:

```rust
use super::ring_buffer::RingBuffer;

#[test]
fn push_appends_within_cap() {
    let mut buf = RingBuffer::with_cap(100);
    buf.push(b"hello ");
    buf.push(b"world");
    let snap = buf.snapshot();
    assert_eq!(snap.bytes, b"hello world".to_vec());
    assert!(!snap.truncated_from_start);
    assert_eq!(snap.current_offset, 11);
}

#[test]
fn total_written_is_monotonic_across_pushes() {
    let mut buf = RingBuffer::with_cap(100);
    buf.push(b"abc");
    buf.push(b"de");
    let snap = buf.snapshot();
    assert_eq!(snap.current_offset, 5);
}
```

Add at bottom of `ring_buffer.rs`:

```rust
#[cfg(test)]
#[path = "ring_buffer_tests.rs"]
mod tests;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p remote_server agent_sessions::ring_buffer`
Expected: FAIL — `RingBuffer` not defined.

- [ ] **Step 3: Implement minimal RingBuffer**

In `ring_buffer.rs`:

```rust
use std::collections::VecDeque;

pub struct RingBuffer {
    buf: VecDeque<u8>,
    cap: usize,
    total_written: u64,
    truncations: u64,
}

pub struct Snapshot {
    pub bytes: Vec<u8>,
    pub truncated_from_start: bool,
    pub current_offset: u64,
}

impl RingBuffer {
    pub fn with_cap(cap: usize) -> Self {
        Self { buf: VecDeque::with_capacity(cap), cap, total_written: 0, truncations: 0 }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes.iter().copied());
        self.total_written += bytes.len() as u64;
        // Truncation handled in Task 1.3.
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            bytes: self.buf.iter().copied().collect(),
            truncated_from_start: self.truncations > 0,
            current_offset: self.total_written,
        }
    }

    pub fn truncations(&self) -> u64 {
        self.truncations
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p remote_server agent_sessions::ring_buffer`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/remote_server/src/agent_sessions/ring_buffer*
git commit -m "Add RingBuffer basic push/snapshot"
```

### Task 1.3: RingBuffer — bounded capacity with ANSI-boundary truncation

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/ring_buffer.rs`
- Modify: `crates/remote_server/src/agent_sessions/ring_buffer_tests.rs`

- [ ] **Step 1: Write failing test for simple truncation**

Add to `ring_buffer_tests.rs`:

```rust
#[test]
fn push_truncates_when_exceeding_cap() {
    let mut buf = RingBuffer::with_cap(10);
    buf.push(b"0123456789");
    buf.push(b"abcde");
    let snap = buf.snapshot();
    assert!(snap.truncated_from_start);
    assert!(snap.bytes.len() <= 10);
    assert!(snap.bytes.ends_with(b"abcde"));
    assert_eq!(snap.current_offset, 15);
}
```

- [ ] **Step 2: Write failing test for ANSI-boundary truncation**

```rust
#[test]
fn truncation_does_not_split_csi_sequence() {
    let mut buf = RingBuffer::with_cap(20);
    // Pre-fill so the next push truncates inside what would be a CSI sequence.
    buf.push(b"AAAAAAAAAAAAAAAAAA"); // 18 bytes
    buf.push(b"\x1b[31mRED\x1b[0mTAIL"); // 16 bytes; total 34, must truncate 14
    let snap = buf.snapshot();
    // The retained buffer must not begin partway through ESC[ ... m.
    // Acceptable starts: literal byte, complete ANSI prefix, or after a final byte.
    assert!(!snap.bytes.starts_with(&[b'[']));
    assert!(!snap.bytes.starts_with(&[b';']));
    // The trailing content must still be there.
    assert!(snap.bytes.ends_with(b"TAIL"));
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p remote_server agent_sessions::ring_buffer`
Expected: two FAILS.

- [ ] **Step 4: Implement bounded push with ANSI-safe truncation**

Replace `push` in `ring_buffer.rs`:

```rust
impl RingBuffer {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes.iter().copied());
        self.total_written += bytes.len() as u64;
        if self.buf.len() > self.cap {
            self.trim_to_cap_at_ansi_boundary();
        }
    }

    fn trim_to_cap_at_ansi_boundary(&mut self) {
        // Drop bytes from the front, advancing to the next safe ANSI boundary.
        // Safe = start of a literal byte run, or just past the final byte of an
        // OSC/CSI sequence.
        let excess = self.buf.len().saturating_sub(self.cap);
        let mut drop = excess;
        // Look ahead from `drop` and advance until at a safe split point.
        while drop < self.buf.len() {
            let b = self.buf[drop];
            // ESC (0x1B) starts a sequence — never split immediately after.
            if b == 0x1B {
                // Skip to next terminator: BEL (0x07), ST (ESC \\), or letter 0x40-0x7E.
                let mut j = drop + 1;
                while j < self.buf.len() {
                    let c = self.buf[j];
                    if c == 0x07 { drop = j + 1; break; }
                    if c == 0x1B && j + 1 < self.buf.len() && self.buf[j + 1] == b'\\' {
                        drop = j + 2; break;
                    }
                    if (0x40..=0x7E).contains(&c) && !matches!(c, b'[' | b']' | b'P' | b'^' | b'_') {
                        drop = j + 1; break;
                    }
                    j += 1;
                }
                if drop < j { drop = j; } // ran off end; drop everything
                break;
            }
            // CSI parameter/intermediate bytes: keep advancing until a final byte.
            if matches!(b, b'[' | b';' | b'?' | b'!' | b' ' | b'"' | b'#' | b'$' | b'(' | b')') {
                drop += 1;
                continue;
            }
            // Otherwise this is a safe split point.
            break;
        }
        drop = drop.min(self.buf.len());
        for _ in 0..drop {
            self.buf.pop_front();
        }
        self.truncations += 1;
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p remote_server agent_sessions::ring_buffer`
Expected: all 4 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add -u
git commit -m "RingBuffer trims at ANSI sequence boundaries"
```

### Task 1.4: RingBuffer — prepend hard reset on truncated snapshots

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/ring_buffer.rs`
- Modify: `crates/remote_server/src/agent_sessions/ring_buffer_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn snapshot_prepends_hard_reset_when_truncated() {
    let mut buf = RingBuffer::with_cap(10);
    buf.push(b"0123456789ABCDE");
    let snap = buf.snapshot();
    assert!(snap.truncated_from_start);
    // Hard reset prefix: ESC c, ESC[?1049l, ESC[2J, ESC[H.
    let expected_prefix = b"\x1bc\x1b[?1049l\x1b[2J\x1b[H";
    assert!(snap.bytes.starts_with(expected_prefix),
        "snapshot did not start with hard-reset prefix: {:?}",
        &snap.bytes[..expected_prefix.len().min(snap.bytes.len())]);
}

#[test]
fn snapshot_does_not_prepend_reset_when_untruncated() {
    let mut buf = RingBuffer::with_cap(100);
    buf.push(b"hello");
    let snap = buf.snapshot();
    assert_eq!(snap.bytes, b"hello".to_vec());
}
```

- [ ] **Step 2: Run tests — verify the first fails**

Run: `cargo test -p remote_server agent_sessions::ring_buffer`
Expected: `snapshot_prepends_hard_reset_when_truncated` FAILS.

- [ ] **Step 3: Update snapshot to prepend reset when truncated**

In `ring_buffer.rs`:

```rust
const HARD_RESET_PREFIX: &[u8] = b"\x1bc\x1b[?1049l\x1b[2J\x1b[H";

impl RingBuffer {
    pub fn snapshot(&self) -> Snapshot {
        let truncated = self.truncations > 0;
        let mut bytes = Vec::with_capacity(
            self.buf.len() + if truncated { HARD_RESET_PREFIX.len() } else { 0 },
        );
        if truncated {
            bytes.extend_from_slice(HARD_RESET_PREFIX);
        }
        bytes.extend(self.buf.iter().copied());
        Snapshot {
            bytes,
            truncated_from_start: truncated,
            current_offset: self.total_written,
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p remote_server agent_sessions::ring_buffer`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "RingBuffer prepends hard-reset on truncated snapshot"
```

### Task 1.5: RingBuffer — OSC straddling boundary preservation test

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/ring_buffer_tests.rs`

- [ ] **Step 1: Write the regression test**

Add to `ring_buffer_tests.rs`:

```rust
#[test]
fn osc_9277_event_is_either_preserved_or_dropped_whole() {
    // Goal: after truncation, the buffer must not contain a partial OSC 9277.
    // A partial sequence would make the parser emit a corrupt event or get stuck.
    let mut buf = RingBuffer::with_cap(50);
    buf.push(&[b'A'; 40]);
    let osc = b"\x1b]9277;{\"event\":\"ToolStart\"}\x07";
    buf.push(osc);
    let snap = buf.snapshot();
    // The snapshot may or may not contain the OSC, but if it does, it's complete.
    let stripped = snap.bytes.iter().copied().collect::<Vec<u8>>();
    let osc_start = stripped.iter().position(|&b| b == 0x1B);
    if let Some(start) = osc_start {
        // From the ESC, scan for the terminator BEL (0x07).
        let terminator = stripped[start..].iter().position(|&b| b == 0x07);
        assert!(terminator.is_some(),
            "snapshot contains a partial OSC sequence — corrupts parser state");
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p remote_server agent_sessions::ring_buffer`
Expected: PASS (or, if it fails, fix the boundary logic in Task 1.3 to extend `j` correctly to terminator).

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "Test: RingBuffer never leaves a partial OSC after truncation"
```

### Task 1.6: AgentSession lifecycle and supervisor task

**Files:**
- Create: `crates/remote_server/src/agent_sessions/session.rs`
- Create: `crates/remote_server/src/agent_sessions/session_tests.rs`

- [ ] **Step 1: Define the AgentSession struct**

In `session.rs`:

```rust
use crate::agent_sessions::ring_buffer::RingBuffer;
use crate::agent_sessions::types::*;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tokio::sync::{broadcast, oneshot, Mutex, RwLock};

pub struct AgentSession {
    pub id: SessionId,
    pub label: RwLock<String>,
    pub meta: SessionMeta,
    pub state: RwLock<SessionState>,
    pub output_buffer: Mutex<RingBuffer>,
    pub output_tx: broadcast::Sender<Vec<u8>>,
    pub attach_lock: Mutex<Option<AttachedClient>>,
    pub last_active: Mutex<SystemTime>,
    _supervisor: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Debug)]
pub struct SessionMeta {
    pub cmd: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub agent_kind: AgentKind,
    pub started_at: SystemTime,
}

pub struct AttachedClient {
    pub client_id: ClientId,
    pub label: String,
    pub last_seen: Mutex<Instant>,
    pub detach_tx: oneshot::Sender<DetachReason>,
}

impl AgentSession {
    pub async fn current_state(&self) -> SessionStateDescriptor {
        let state = self.state.read().await;
        match &*state {
            SessionState::Running { pid } => SessionStateDescriptor::Running { pid: *pid },
            SessionState::Ended { ended_at, exit } => {
                SessionStateDescriptor::Ended { ended_at: *ended_at, exit: *exit }
            }
        }
    }

    pub async fn summary(self: &Arc<Self>) -> SessionSummary {
        let label = self.label.read().await.clone();
        let state = self.current_state().await;
        let last_active = *self.last_active.lock().await;
        let attached_client_label = self
            .attach_lock
            .lock()
            .await
            .as_ref()
            .map(|c| c.label.clone());
        SessionSummary {
            id: self.id.clone(),
            label,
            agent_kind: self.meta.agent_kind,
            cwd: self.meta.cwd.clone(),
            started_at: self.meta.started_at,
            last_active,
            state,
            attached_client_label,
        }
    }
}
```

- [ ] **Step 2: Add module include in `mod.rs`**

Already done in Task 1.1; just verify `pub mod session;` is present.

- [ ] **Step 3: Add unit test for `summary()` shape**

In `session_tests.rs`:

```rust
use super::session::*;
use super::types::*;
use super::ring_buffer::RingBuffer;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{broadcast, Mutex, RwLock};

fn make_test_session() -> Arc<AgentSession> {
    let (output_tx, _) = broadcast::channel(16);
    let supervisor = tokio::spawn(async {});
    Arc::new(AgentSession {
        id: SessionId("abc".into()),
        label: RwLock::new("test".into()),
        meta: SessionMeta {
            cmd: "/bin/true".into(),
            args: vec![],
            cwd: "/tmp".into(),
            agent_kind: AgentKind::Custom,
            started_at: SystemTime::now(),
        },
        state: RwLock::new(SessionState::Running { pid: 1234 }),
        output_buffer: Mutex::new(RingBuffer::with_cap(64)),
        output_tx,
        attach_lock: Mutex::new(None),
        last_active: Mutex::new(SystemTime::now()),
        _supervisor: supervisor,
    })
}

#[tokio::test]
async fn summary_reflects_running_state() {
    let s = make_test_session();
    let sum = s.summary().await;
    assert_eq!(sum.id, SessionId("abc".into()));
    assert!(matches!(sum.state, SessionStateDescriptor::Running { pid: 1234 }));
    assert!(sum.attached_client_label.is_none());
}
```

Wire it from `session.rs`:

```rust
#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p remote_server agent_sessions::session`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/remote_server/src/agent_sessions/session*
git commit -m "AgentSession type with summary()"
```

### Task 1.7: AgentSessionManager::start — happy path with `bash -c 'true'`

**Files:**
- Create: `crates/remote_server/src/agent_sessions/manager.rs`
- Create: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Write failing test**

In `manager_tests.rs`:

```rust
use super::manager::AgentSessionManager;
use super::types::*;

#[tokio::test]
async fn start_succeeds_with_valid_command_and_cwd() {
    let mgr = AgentSessionManager::new();
    let req = StartRequest {
        requested_label: None,
        cwd: "/tmp".into(),
        agent_kind: AgentKind::Custom,
        cmd: "bash".into(),
        args: vec!["-c".into(), "echo hi; sleep 0.1".into()],
    };
    let started = mgr.start(req).await.expect("start should succeed");
    assert!(!started.id.0.is_empty());
    let list = mgr.list().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, started.id);
}

#[tokio::test]
async fn start_fails_with_invalid_cwd() {
    let mgr = AgentSessionManager::new();
    let req = StartRequest {
        requested_label: None,
        cwd: "/this/path/does/not/exist".into(),
        agent_kind: AgentKind::Custom,
        cmd: "bash".into(),
        args: vec!["-c".into(), "true".into()],
    };
    let res = mgr.start(req).await;
    assert!(matches!(res, Err(StartError::CwdInvalid(_))));
}

#[tokio::test]
async fn start_fails_with_command_not_found() {
    let mgr = AgentSessionManager::new();
    let req = StartRequest {
        requested_label: None,
        cwd: "/tmp".into(),
        agent_kind: AgentKind::Custom,
        cmd: "this_command_definitely_does_not_exist_xyz".into(),
        args: vec![],
    };
    let res = mgr.start(req).await;
    assert!(matches!(res, Err(StartError::CommandNotFound(_))));
}
```

- [ ] **Step 2: Run tests — verify they fail (no impl)**

Run: `cargo test -p remote_server agent_sessions::manager`
Expected: compile errors / undefined types.

- [ ] **Step 3: Define StartRequest and implement AgentSessionManager::new + start**

In `manager.rs`:

```rust
use crate::agent_sessions::ring_buffer::RingBuffer;
use crate::agent_sessions::session::*;
use crate::agent_sessions::types::*;
use crate::agent_sessions::{RING_BUFFER_CAP_BYTES, MAX_ENDED_SESSIONS};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{broadcast, Mutex, RwLock};

pub struct AgentSessionManager {
    sessions: RwLock<HashMap<SessionId, Arc<AgentSession>>>,
    ended_lru: Mutex<VecDeque<SessionId>>,
}

pub struct StartRequest {
    pub requested_label: Option<String>,
    pub cwd: String,
    pub agent_kind: AgentKind,
    pub cmd: String,
    pub args: Vec<String>,
}

pub struct StartedSession {
    pub id: SessionId,
    pub label: String,
    pub started_at: SystemTime,
}

impl AgentSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            ended_lru: Mutex::new(VecDeque::new()),
        }
    }

    pub async fn start(&self, req: StartRequest) -> Result<StartedSession, StartError> {
        if !Path::new(&req.cwd).is_dir() {
            return Err(StartError::CwdInvalid(req.cwd));
        }

        let mut command = CommandBuilder::new(&req.cmd);
        for a in &req.args { command.arg(a); }
        command.cwd(&req.cwd);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| StartError::OsError(e.to_string()))?;

        let child = pair.slave.spawn_command(command).map_err(|e| {
            let s = e.to_string();
            if s.contains("No such file or directory") || s.contains("not found") {
                StartError::CommandNotFound(req.cmd.clone())
            } else {
                StartError::OsError(s)
            }
        })?;

        let id = SessionId::new();
        let started_at = SystemTime::now();
        let label = req.requested_label.unwrap_or_else(|| {
            format!("{:?}: {}", req.agent_kind, req.cwd)
        });

        let (output_tx, _) = broadcast::channel(64);
        let supervisor = tokio::spawn(async move {
            // Real read loop comes in Task 1.8.
            let _ = child;
        });

        let session = Arc::new(AgentSession {
            id: id.clone(),
            label: RwLock::new(label.clone()),
            meta: SessionMeta {
                cmd: req.cmd,
                args: req.args,
                cwd: req.cwd,
                agent_kind: req.agent_kind,
                started_at,
            },
            state: RwLock::new(SessionState::Running { pid: 0 }), // pid wired in Task 1.8
            output_buffer: Mutex::new(RingBuffer::with_cap(RING_BUFFER_CAP_BYTES)),
            output_tx,
            attach_lock: Mutex::new(None),
            last_active: Mutex::new(started_at),
            _supervisor: supervisor,
        });

        self.sessions.write().await.insert(id.clone(), session);

        Ok(StartedSession { id, label, started_at })
    }

    pub async fn list(&self) -> Vec<SessionSummary> {
        let sessions = self.sessions.read().await;
        let mut out = Vec::with_capacity(sessions.len());
        for s in sessions.values() {
            out.push(s.summary().await);
        }
        out
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p remote_server agent_sessions::manager`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/remote_server/src/agent_sessions/manager*
git commit -m "AgentSessionManager::start with cwd/command validation"
```

### Task 1.8: Supervisor task — wire PTY reads to buffer and broadcast

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test]
async fn output_from_child_lands_in_ring_buffer() {
    let mgr = AgentSessionManager::new();
    let req = StartRequest {
        requested_label: None,
        cwd: "/tmp".into(),
        agent_kind: AgentKind::Custom,
        cmd: "bash".into(),
        args: vec!["-c".into(), "echo MARKER; sleep 0.1".into()],
    };
    let started = mgr.start(req).await.unwrap();
    // Allow time for the child to write.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let sessions = mgr.sessions.read().await;
    let session = sessions.get(&started.id).unwrap();
    let buf = session.output_buffer.lock().await;
    let snap = buf.snapshot();
    assert!(snap.bytes.windows(b"MARKER".len()).any(|w| w == b"MARKER"),
        "buffer did not contain MARKER: {:?}", String::from_utf8_lossy(&snap.bytes));
}
```

Note: this test reads `mgr.sessions` directly. Make the field `pub(crate)` temporarily; revisit visibility later.

- [ ] **Step 2: Run test — verify FAIL**

Run: `cargo test -p remote_server agent_sessions::manager output_from_child`
Expected: FAIL — no output captured.

- [ ] **Step 3: Replace supervisor task with real read loop**

In `manager.rs`, replace the supervisor body inside `start`:

```rust
let mut reader = pair.master.try_clone_reader().map_err(|e| StartError::OsError(e.to_string()))?;
let buffer_clone = Arc::clone(&session); // before insertion — restructure as needed
let tx_clone = output_tx.clone();

let supervisor = tokio::spawn(async move {
    let mut buf = [0u8; 4096];
    loop {
        let n = match tokio::task::spawn_blocking({
            let r = reader.as_mut();
            move || r.read(&mut buf)
        }).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(n)) => n,
            Ok(Err(_)) => break,
        };
        let chunk = buf[..n].to_vec();
        {
            let mut rb = buffer_clone.output_buffer.lock().await;
            rb.push(&chunk);
        }
        let _ = tx_clone.send(chunk);
        *buffer_clone.last_active.lock().await = SystemTime::now();
    }
    // Mark Ended.
    let exit = ExitDescriptor::Code(0);
    *buffer_clone.state.write().await = SessionState::Ended { exit, ended_at: SystemTime::now() };
});
```

(You will need to restructure the construction order: build the `AgentSession` first, insert into the map, then spawn the supervisor with `Arc<AgentSession>`. Use `Arc::clone(&session)` for the supervisor. The exact `try_clone_reader` API on `portable-pty` may differ — adjust to match the installed version.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p remote_server agent_sessions::manager`
Expected: 4 tests PASS, including new one.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "AgentSessionManager supervisor reads PTY into ring buffer"
```

### Task 1.9: Attach — first attach, snapshot then live

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test]
async fn first_attach_receives_snapshot_then_live() {
    use tokio_stream::StreamExt;

    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None,
        cwd: "/tmp".into(),
        agent_kind: AgentKind::Custom,
        cmd: "bash".into(),
        args: vec!["-c".into(), "echo PRE; sleep 0.1; echo POST; sleep 1".into()],
    };
    let started = mgr.start(req).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let client = ClientId("client-A".into());
    let mut stream = mgr.attach(started.id.clone(), client, "client-A".into(), None, 80, 24)
        .await
        .unwrap();

    let first = stream.next().await.unwrap();
    let AttachEvent::Snapshot { bytes, .. } = first else { panic!("expected Snapshot first") };
    assert!(bytes.windows(b"PRE".len()).any(|w| w == b"PRE"));

    // Look for POST in either the snapshot or a subsequent live event (timing dependent).
    let has_post = bytes.windows(b"POST".len()).any(|w| w == b"POST") || {
        let mut found = false;
        while let Some(ev) = stream.next().await {
            if let AttachEvent::Live { chunk, .. } = ev {
                if chunk.windows(b"POST".len()).any(|w| w == b"POST") {
                    found = true;
                    break;
                }
            }
        }
        found
    };
    assert!(has_post);
}
```

- [ ] **Step 2: Run — verify FAIL**

Run: `cargo test -p remote_server first_attach_receives`
Expected: FAIL.

- [ ] **Step 3: Implement attach**

Define `AttachEvent` in `types.rs`:

```rust
#[derive(Clone, Debug)]
pub enum AttachEvent {
    Snapshot { bytes: Vec<u8>, truncated_from_start: bool, current_offset: u64 },
    Live { chunk: Vec<u8>, offset: u64 },
    Detached { reason: DetachReason, superseding_client_label: Option<String> },
    SessionEnded { exit: ExitDescriptor, ended_at: SystemTime },
}
```

In `manager.rs` add:

```rust
use futures::Stream;
use std::pin::Pin;
use tokio::sync::oneshot;

pub type AttachStream = Pin<Box<dyn Stream<Item = AttachEvent> + Send + 'static>>;

impl AgentSessionManager {
    pub async fn attach(
        self: &Arc<Self>,
        id: SessionId,
        client_id: ClientId,
        client_label: String,
        _from_offset: Option<u64>,
        cols: u16,
        rows: u16,
    ) -> Result<AttachStream, AttachError> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(AttachError::NotFound)?.clone();
        drop(sessions);

        // Resize PTY to client's window.
        // (Add resize wiring later in Task 1.13; ignore here.)
        let _ = (cols, rows);

        // Subscribe to live broadcast first to avoid losing chunks between snapshot and live.
        let mut rx = session.output_tx.subscribe();

        // Take the snapshot.
        let snap = {
            let buf = session.output_buffer.lock().await;
            buf.snapshot()
        };

        // Install / kick attach lock.
        let (detach_tx, mut detach_rx) = oneshot::channel();
        let new_attached = AttachedClient {
            client_id: client_id.clone(),
            label: client_label.clone(),
            last_seen: Mutex::new(Instant::now()),
            detach_tx,
        };
        let prev = {
            let mut lock = session.attach_lock.lock().await;
            lock.replace(new_attached)
        };
        if let Some(prev) = prev {
            let _ = prev.detach_tx.send(DetachReason::Superseded);
        }

        // Drive the stream as a generator: emit Snapshot, then forward Live chunks until detached or ended.
        let session_clone = session.clone();
        let stream = async_stream::stream! {
            yield AttachEvent::Snapshot {
                bytes: snap.bytes,
                truncated_from_start: snap.truncated_from_start,
                current_offset: snap.current_offset,
            };

            let mut next_offset = snap.current_offset;
            loop {
                tokio::select! {
                    chunk = rx.recv() => match chunk {
                        Ok(c) => {
                            next_offset += c.len() as u64;
                            yield AttachEvent::Live { chunk: c, offset: next_offset };
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break,
                    },
                    reason = &mut detach_rx => {
                        let reason = reason.unwrap_or(DetachReason::ClientRequested);
                        yield AttachEvent::Detached { reason, superseding_client_label: None };
                        return;
                    }
                }
                // Check if session ended.
                if let SessionState::Ended { exit, ended_at } = *session_clone.state.read().await {
                    yield AttachEvent::SessionEnded { exit, ended_at };
                    return;
                }
            }
        };

        Ok(Box::pin(stream))
    }
}
```

(`async_stream::stream!` requires the `async-stream` crate dep. Add `async-stream = "0.3"` to `crates/remote_server/Cargo.toml`.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p remote_server first_attach`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "AgentSessionManager::attach with snapshot+live stream"
```

### Task 1.10: Kick semantics — second attach displaces first

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn second_attach_kicks_first_with_superseded() {
    use tokio_stream::StreamExt;

    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None,
        cwd: "/tmp".into(),
        agent_kind: AgentKind::Custom,
        cmd: "bash".into(),
        args: vec!["-c".into(), "sleep 5".into()],
    };
    let started = mgr.start(req).await.unwrap();

    let mut a = mgr.attach(started.id.clone(), ClientId("A".into()), "A".into(), None, 80, 24).await.unwrap();
    // Drain snapshot.
    let _ = a.next().await;

    let _b = mgr.attach(started.id.clone(), ClientId("B".into()), "B".into(), None, 80, 24).await.unwrap();

    // A should receive Detached { reason: Superseded } now.
    let ev = tokio::time::timeout(std::time::Duration::from_secs(2), a.next()).await.unwrap().unwrap();
    assert!(matches!(ev, AttachEvent::Detached { reason: DetachReason::Superseded, .. }));
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p remote_server second_attach_kicks_first`
Expected: PASS (the kick is already wired in Task 1.9).

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "Test: second attach kicks first with Superseded"
```

### Task 1.11: Heartbeat — timeout sweep releases stale attach locks

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Add `heartbeat` and a manual `sweep_for_test` API**

In `manager.rs`:

```rust
use std::time::{Duration, Instant};
use crate::agent_sessions::HEARTBEAT_TIMEOUT_SECS;

impl AgentSessionManager {
    pub async fn heartbeat(&self, id: SessionId, client_id: ClientId) -> Result<(), HeartbeatError> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(HeartbeatError::NotFound)?;
        let lock = session.attach_lock.lock().await;
        match lock.as_ref() {
            Some(c) if c.client_id == client_id => {
                *c.last_seen.lock().await = Instant::now();
                Ok(())
            }
            _ => Err(HeartbeatError::NotAttached),
        }
    }

    /// Sweep all sessions and release attach locks whose `last_seen` is older than the timeout.
    /// In production a background task calls this periodically; tests call it directly.
    pub async fn sweep_heartbeats(&self) {
        let now = Instant::now();
        let timeout = Duration::from_secs(HEARTBEAT_TIMEOUT_SECS);
        let sessions = self.sessions.read().await;
        for session in sessions.values() {
            let mut lock = session.attach_lock.lock().await;
            let stale = match lock.as_ref() {
                Some(c) => now.duration_since(*c.last_seen.lock().await) > timeout,
                None => false,
            };
            if stale {
                if let Some(c) = lock.take() {
                    let _ = c.detach_tx.send(DetachReason::HeartbeatTimeout);
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HeartbeatError {
    #[error("session not found")]
    NotFound,
    #[error("not attached")]
    NotAttached,
}
```

- [ ] **Step 2: Write tests**

In `manager_tests.rs`:

```rust
#[tokio::test]
async fn heartbeat_timeout_releases_lock() {
    use tokio_stream::StreamExt;
    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None, cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(), args: vec!["-c".into(), "sleep 5".into()],
    };
    let started = mgr.start(req).await.unwrap();
    let mut a = mgr.attach(started.id.clone(), ClientId("A".into()), "A".into(), None, 80, 24).await.unwrap();
    let _ = a.next().await; // snapshot

    // Force stale: manually backdate last_seen.
    {
        let sessions = mgr.sessions.read().await;
        let session = sessions.get(&started.id).unwrap();
        let lock = session.attach_lock.lock().await;
        if let Some(c) = lock.as_ref() {
            let mut ls = c.last_seen.lock().await;
            *ls = Instant::now() - Duration::from_secs(HEARTBEAT_TIMEOUT_SECS + 5);
        }
    }
    mgr.sweep_heartbeats().await;

    let ev = tokio::time::timeout(Duration::from_secs(2), a.next()).await.unwrap().unwrap();
    assert!(matches!(ev, AttachEvent::Detached { reason: DetachReason::HeartbeatTimeout, .. }));
}

#[tokio::test]
async fn heartbeat_refreshes_last_seen() {
    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None, cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(), args: vec!["-c".into(), "sleep 5".into()],
    };
    let started = mgr.start(req).await.unwrap();
    let _stream = mgr.attach(started.id.clone(), ClientId("A".into()), "A".into(), None, 80, 24).await.unwrap();

    // Heartbeat should succeed.
    mgr.heartbeat(started.id.clone(), ClientId("A".into())).await.unwrap();
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p remote_server heartbeat`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "AgentSessionManager heartbeat + timeout sweep"
```

### Task 1.12: Kill — SIGTERM with SIGKILL escalation after 5s

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test]
async fn kill_sends_sigterm_then_sigkill_after_5s() {
    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None, cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(),
        args: vec!["-c".into(), "trap '' TERM; sleep 30".into()],
    };
    let started = mgr.start(req).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let kill_start = Instant::now();
    mgr.kill(started.id.clone()).await.unwrap();
    let elapsed = kill_start.elapsed();
    assert!(elapsed >= Duration::from_secs(5));
    assert!(elapsed < Duration::from_secs(7));
}
```

- [ ] **Step 2: Implement `kill`**

In `manager.rs`:

```rust
use std::process::ExitStatus;

impl AgentSessionManager {
    pub async fn kill(&self, id: SessionId) -> Result<(), KillError> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(&id).ok_or(KillError::NotFound)?.clone();
        drop(sessions);

        // Take the PID under the running state.
        let pid = match *session.state.read().await {
            SessionState::Running { pid } => pid,
            SessionState::Ended { .. } => return Ok(()),
        };

        // SIGTERM
        unsafe { libc::kill(pid as i32, libc::SIGTERM); }

        // Wait up to 5s for the child to exit (poll state).
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            if matches!(*session.state.read().await, SessionState::Ended { .. }) {
                return Ok(());
            }
            if Instant::now() >= deadline { break; }
        }

        // Escalate to SIGKILL
        unsafe { libc::kill(pid as i32, libc::SIGKILL); }
        // Best-effort wait.
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KillError {
    #[error("session not found")]
    NotFound,
}
```

Note: `pid` must actually be populated. Update Task 1.7/1.8 wiring so `state` is `Running { pid: child_pid }` where `child_pid` comes from `child.process_id()` on `portable-pty`'s `Child`. (Inspect the actual API at implementation time.)

- [ ] **Step 3: Run tests**

Run: `cargo test -p remote_server kill_sends_sigterm`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "AgentSessionManager::kill with SIGTERM then SIGKILL escalation"
```

### Task 1.13: Resize, send_input, detach, rename — straightforward primitives

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`
- Modify: `crates/remote_server/src/agent_sessions/session.rs` (add pty_master field, accessor)

- [ ] **Step 1: Store PTY master on AgentSession**

Extend `AgentSession` with `pub pty_master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>` and populate it in `start`.

- [ ] **Step 2: Write tests**

```rust
#[tokio::test]
async fn send_input_writes_to_pty() {
    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None, cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(),
        args: vec!["-c".into(), "read X && echo got:$X; sleep 0.2".into()],
    };
    let started = mgr.start(req).await.unwrap();
    let mut a = mgr.attach(started.id.clone(), ClientId("A".into()), "A".into(), None, 80, 24).await.unwrap();
    let _ = a.next().await; // snapshot

    mgr.send_input(started.id.clone(), ClientId("A".into()), b"hello\n".to_vec()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let buf = {
        let sessions = mgr.sessions.read().await;
        let s = sessions.get(&started.id).unwrap();
        s.output_buffer.lock().await.snapshot()
    };
    assert!(buf.bytes.windows(b"got:hello".len()).any(|w| w == b"got:hello"));
}

#[tokio::test]
async fn rename_updates_label() {
    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None, cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(), args: vec!["-c".into(), "sleep 5".into()],
    };
    let started = mgr.start(req).await.unwrap();
    mgr.rename(started.id.clone(), "new label".into()).await.unwrap();
    let list = mgr.list().await;
    assert_eq!(list[0].label, "new label");
}

#[tokio::test]
async fn rename_rejects_empty_label() {
    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None, cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(), args: vec!["-c".into(), "sleep 5".into()],
    };
    let started = mgr.start(req).await.unwrap();
    let res = mgr.rename(started.id.clone(), "".into()).await;
    assert!(res.is_err());
}
```

- [ ] **Step 3: Implement**

```rust
impl AgentSessionManager {
    pub async fn send_input(&self, id: SessionId, client_id: ClientId, bytes: Vec<u8>) -> Result<(), InputError> {
        let sessions = self.sessions.read().await;
        let s = sessions.get(&id).ok_or(InputError::NotFound)?;
        let lock = s.attach_lock.lock().await;
        let attached = lock.as_ref().ok_or(InputError::NotAttached)?;
        if attached.client_id != client_id { return Err(InputError::NotAttached); }
        drop(lock);
        if matches!(*s.state.read().await, SessionState::Ended { .. }) {
            return Err(InputError::SessionEnded);
        }
        let mut master = s.pty_master.lock().await;
        if let Some(m) = master.as_mut() {
            use std::io::Write;
            m.take_writer().map_err(|e| InputError::IoError(e.to_string()))?
                .write_all(&bytes).map_err(|e| InputError::IoError(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn resize(&self, id: SessionId, cols: u16, rows: u16) -> Result<(), ResizeError> {
        let sessions = self.sessions.read().await;
        let s = sessions.get(&id).ok_or(ResizeError::NotFound)?;
        let mut master = s.pty_master.lock().await;
        if let Some(m) = master.as_mut() {
            m.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
                .map_err(|e| ResizeError::IoError(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn detach(&self, id: SessionId, client_id: ClientId) -> Result<(), DetachError> {
        let sessions = self.sessions.read().await;
        let s = sessions.get(&id).ok_or(DetachError::NotFound)?;
        let mut lock = s.attach_lock.lock().await;
        let take = match lock.as_ref() {
            Some(c) if c.client_id == client_id => true,
            _ => false,
        };
        if take {
            if let Some(c) = lock.take() {
                let _ = c.detach_tx.send(DetachReason::ClientRequested);
            }
        }
        Ok(())
    }

    pub async fn rename(&self, id: SessionId, new_label: String) -> Result<(), RenameError> {
        if new_label.trim().is_empty() { return Err(RenameError::EmptyLabel); }
        let sessions = self.sessions.read().await;
        let s = sessions.get(&id).ok_or(RenameError::NotFound)?;
        *s.label.write().await = new_label;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("session not found")] NotFound,
    #[error("not attached")] NotAttached,
    #[error("session ended")] SessionEnded,
    #[error("io: {0}")] IoError(String),
}
#[derive(Debug, thiserror::Error)]
pub enum ResizeError { #[error("session not found")] NotFound, #[error("io: {0}")] IoError(String) }
#[derive(Debug, thiserror::Error)]
pub enum DetachError { #[error("session not found")] NotFound }
#[derive(Debug, thiserror::Error)]
pub enum RenameError { #[error("session not found")] NotFound, #[error("empty label")] EmptyLabel }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p remote_server agent_sessions::manager`
Expected: PASS (all new + previous).

- [ ] **Step 5: Commit**

```bash
git add -u
git commit -m "AgentSessionManager: send_input, resize, detach, rename"
```

### Task 1.14: LRU eviction for Ended sessions

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test]
async fn ended_sessions_evicted_after_max() {
    let mgr = Arc::new(AgentSessionManager::new());
    // Run MAX_ENDED_SESSIONS + 1 quick sessions.
    for i in 0..(MAX_ENDED_SESSIONS + 1) {
        let req = StartRequest {
            requested_label: Some(format!("s{i}")),
            cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
            cmd: "bash".into(), args: vec!["-c".into(), "true".into()],
        };
        let _ = mgr.start(req).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Allow supervisors to mark Ended.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let list = mgr.list().await;
    let ended_count = list.iter().filter(|s| matches!(s.state, SessionStateDescriptor::Ended { .. })).count();
    assert!(ended_count <= MAX_ENDED_SESSIONS);
}

#[tokio::test]
async fn running_session_is_never_evicted_even_if_oldest() {
    let mgr = Arc::new(AgentSessionManager::new());
    // Start one long-running.
    let long_req = StartRequest {
        requested_label: Some("LONG".into()),
        cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(), args: vec!["-c".into(), "sleep 10".into()],
    };
    let long = mgr.start(long_req).await.unwrap();

    // Now fill ended LRU.
    for i in 0..(MAX_ENDED_SESSIONS + 2) {
        let req = StartRequest {
            requested_label: Some(format!("s{i}")),
            cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
            cmd: "bash".into(), args: vec!["-c".into(), "true".into()],
        };
        let _ = mgr.start(req).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let list = mgr.list().await;
    assert!(list.iter().any(|s| s.id == long.id), "long-running session was evicted");
}
```

- [ ] **Step 2: Implement eviction in the supervisor's "session ended" path**

When the supervisor transitions a session to `Ended`, it should:
1. Push the session id to `ended_lru` back.
2. If `ended_lru.len() > MAX_ENDED_SESSIONS`, pop the front and remove it from `sessions`.

In the supervisor's exit block:

```rust
{
    let mut ended = mgr_clone.ended_lru.lock().await;
    ended.push_back(id_clone.clone());
    while ended.len() > MAX_ENDED_SESSIONS {
        if let Some(oldest) = ended.pop_front() {
            let mut sessions = mgr_clone.sessions.write().await;
            if let Some(s) = sessions.get(&oldest) {
                let is_running = matches!(*s.state.read().await, SessionState::Running { .. });
                if !is_running { sessions.remove(&oldest); }
                else { ended.push_back(oldest); } // shouldn't be here; defensive
            }
        }
    }
}
```

(You will need to give the supervisor access to a `Weak<AgentSessionManager>` or refactor so the manager owns the supervisor's lifecycle. The cleanest path: have the supervisor task hold `Weak<AgentSessionManager>`; in `start`, spawn with `Weak`. Upgrade on use.)

- [ ] **Step 3: Run tests**

Run: `cargo test -p remote_server ended_sessions_evicted`
Expected: PASS for both tests.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "Evict oldest Ended sessions when MAX_ENDED_SESSIONS exceeded"
```

### Task 1.15: Background heartbeat sweep task

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`

- [ ] **Step 1: Add a constructor that spawns the sweep**

```rust
impl AgentSessionManager {
    pub fn start_background_sweeps(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_SWEEP_INTERVAL_SECS));
            loop {
                interval.tick().await;
                let Some(mgr) = me.upgrade() else { break; };
                mgr.sweep_heartbeats().await;
            }
        });
    }
}
```

- [ ] **Step 2: Call `start_background_sweeps` from daemon main**

Find the daemon entry point (`crates/remote_server/src/bin/...` or wherever the daemon starts up — `grep -rn "fn main" crates/remote_server/`). After constructing `AgentSessionManager`, call `mgr.start_background_sweeps()`.

- [ ] **Step 3: Verify build**

Run: `cargo build -p remote_server`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "Spawn background heartbeat sweep on daemon startup"
```

### Task 1.16: Inspect RPC implementation

**Files:**
- Modify: `crates/remote_server/src/agent_sessions/manager.rs`
- Modify: `crates/remote_server/src/agent_sessions/manager_tests.rs`

- [ ] **Step 1: Implement `inspect`**

```rust
impl AgentSessionManager {
    pub async fn inspect(&self, id: SessionId) -> Result<SessionInspection, InspectError> {
        let sessions = self.sessions.read().await;
        let s = sessions.get(&id).ok_or(InspectError::NotFound)?;
        let buf = s.output_buffer.lock().await;
        let lock = s.attach_lock.lock().await;
        let last_heartbeat = match lock.as_ref() {
            Some(c) => Some(*c.last_seen.lock().await),
            None => None,
        };
        Ok(SessionInspection {
            ring_buffer_bytes: buf.snapshot().bytes.len(),
            ring_buffer_truncations: buf.truncations(),
            attach_history: vec![], // history tracking is a follow-up; v1 returns empty
            last_heartbeat,
            child_state: s.current_state().await,
        })
    }
}

pub struct SessionInspection {
    pub ring_buffer_bytes: usize,
    pub ring_buffer_truncations: u64,
    pub attach_history: Vec<AttachHistoryEntry>,
    pub last_heartbeat: Option<Instant>,
    pub child_state: SessionStateDescriptor,
}

#[derive(Clone, Debug)]
pub struct AttachHistoryEntry {
    pub client_label: String,
    pub attached_at: SystemTime,
    pub detached_at: Option<SystemTime>,
    pub detach_reason: Option<DetachReason>,
}

#[derive(Debug, thiserror::Error)]
pub enum InspectError { #[error("session not found")] NotFound }
```

- [ ] **Step 2: Test**

```rust
#[tokio::test]
async fn inspect_returns_buffer_stats() {
    let mgr = Arc::new(AgentSessionManager::new());
    let req = StartRequest {
        requested_label: None, cwd: "/tmp".into(), agent_kind: AgentKind::Custom,
        cmd: "bash".into(), args: vec!["-c".into(), "echo X; sleep 0.2".into()],
    };
    let started = mgr.start(req).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let insp = mgr.inspect(started.id).await.unwrap();
    assert!(insp.ring_buffer_bytes > 0);
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p remote_server inspect_returns_buffer_stats
git add -u
git commit -m "AgentSessionManager::inspect for debug RPC"
```

### Task 1.17: Server-side RPC handlers

**Files:**
- Create: `crates/remote_server/src/server_handlers/agent_sessions.rs`
- Modify: `crates/remote_server/src/server_handlers/mod.rs` (or equivalent dispatch site — find via `grep -rn "match.*ClientMessage" crates/remote_server/src/`)

- [ ] **Step 1: Write the handler module**

```rust
use crate::agent_sessions::{
    AgentSessionManager, types::*, manager::{StartRequest, AttachStream}
};
use crate::proto;
use std::sync::Arc;

pub async fn handle_start(
    mgr: &Arc<AgentSessionManager>,
    req: proto::StartAgentSessionRequest,
) -> proto::StartAgentSessionResponse {
    let kind = agent_kind_from_proto(req.agent_kind);
    let (cmd, args) = match kind {
        AgentKind::Claude => ("claude".into(), vec![]),
        AgentKind::Codex => ("codex".into(), vec![]),
        AgentKind::Gemini => ("gemini".into(), vec![]),
        AgentKind::OpenCode => ("opencode".into(), vec![]),
        AgentKind::Custom => {
            (req.custom_command.unwrap_or_default(), req.custom_args)
        }
    };
    let sr = StartRequest {
        requested_label: req.requested_label,
        cwd: req.cwd,
        agent_kind: kind,
        cmd,
        args,
    };
    match mgr.start(sr).await {
        Ok(started) => proto::StartAgentSessionResponse {
            result: Some(proto::start_agent_session_response::Result::Started(
                proto::StartedSession {
                    session_id: started.id.0,
                    label: started.label,
                    started_at_unix_ms: time_to_ms(started.started_at),
                },
            )),
        },
        Err(e) => proto::StartAgentSessionResponse {
            result: Some(proto::start_agent_session_response::Result::Error(
                start_error_to_proto(e),
            )),
        },
    }
}

fn agent_kind_from_proto(n: i32) -> AgentKind {
    match proto::AgentKind::try_from(n).unwrap_or(proto::AgentKind::Unspecified) {
        proto::AgentKind::Claude => AgentKind::Claude,
        proto::AgentKind::Codex => AgentKind::Codex,
        proto::AgentKind::Gemini => AgentKind::Gemini,
        proto::AgentKind::Opencode => AgentKind::OpenCode,
        proto::AgentKind::Custom | proto::AgentKind::Unspecified => AgentKind::Custom,
    }
}

fn time_to_ms(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn start_error_to_proto(e: StartError) -> proto::StartError {
    use proto::start_error::Kind;
    let (kind, detail) = match e {
        StartError::CommandNotFound(s) => (Kind::CommandNotFound, s),
        StartError::CwdInvalid(s) => (Kind::CwdInvalid, s),
        StartError::OsError(s) => (Kind::OsError, s),
        StartError::Unsupported => (Kind::Unsupported, "".into()),
    };
    proto::StartError { kind: kind as i32, detail }
}

// Similar handlers for list, attach (streaming!), detach, kill, rename, input, resize, inspect.
// Attach streams events back as ServerMessage::AttachAgentSessionEvent push messages.
```

(Write the other handlers analogously. For attach: the handler holds the `AttachStream`, polls it, and pushes each event over the server's outgoing message channel. The push mechanism already exists in `remote_server` — find it via `grep -rn "send_push\|push_message\|broadcast" crates/remote_server/src/`.)

- [ ] **Step 2: Wire into the dispatch**

In the file that pattern-matches `ClientMessage::Message`, add an arm per new request that calls into the handler.

- [ ] **Step 3: Build**

Run: `cargo build -p remote_server`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
git add crates/remote_server/src/server_handlers/
git commit -m "Wire agent session RPCs to AgentSessionManager"
```

### Task 1.18: Phase 1 verification

- [ ] **Step 1: Run the full agent_sessions test suite**

Run: `cargo nextest run -p remote_server agent_sessions`
Expected: all tests pass.

- [ ] **Step 2: Run protocol tests**

Run: `cargo nextest run -p remote_server protocol_tests`
Expected: pass.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy -p remote_server --all-targets -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Run fmt**

Run: `cargo fmt --check`
Expected: clean.

- [ ] **Step 5: Tag the phase**

```bash
git tag phase-1-daemon-complete
```

---

## Phase 2 — Client: panes, list view, resolver, registry, heartbeat

After Phase 1 (or in parallel against Phase 0). Behind `FeatureFlag::RemoteAgentSessions`.

### Task 2.1: External-editor URL helper

**Files:**
- Create: `app/src/external_editor.rs`
- Create: `app/src/external_editor_tests.rs`
- Modify: `app/src/lib.rs` (add `pub mod external_editor;`)

- [ ] **Step 1: Write failing tests**

In `external_editor_tests.rs`:

```rust
use super::external_editor::*;

#[test]
fn vscode_url_simple_path() {
    let url = remote_editor_url(ExternalEditor::VsCode, "myhost", "/home/me/proj");
    assert_eq!(url, "vscode://vscode-remote/ssh-remote+myhost/home/me/proj");
}

#[test]
fn cursor_scheme() {
    let url = remote_editor_url(ExternalEditor::Cursor, "host", "/p");
    assert!(url.starts_with("cursor://"));
}

#[test]
fn path_with_spaces_is_percent_encoded() {
    let url = remote_editor_url(ExternalEditor::VsCode, "host", "/home/me/my project");
    assert!(url.contains("%20"));
    assert!(!url.contains(" "));
}

#[test]
fn path_with_special_chars_is_encoded() {
    let url = remote_editor_url(ExternalEditor::VsCode, "host", "/p/a#b?c");
    assert!(!url.contains("#"));
    assert!(!url.contains("?"));
}

#[test]
fn unicode_path_is_encoded() {
    let url = remote_editor_url(ExternalEditor::VsCode, "host", "/п/привет");
    assert!(!url.chars().any(|c| !c.is_ascii()));
}
```

- [ ] **Step 2: Run tests — verify they fail**

Run: `cargo test -p warp external_editor`
Expected: FAIL.

- [ ] **Step 3: Implement**

In `external_editor.rs`:

```rust
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExternalEditor {
    VsCode,
    Cursor,
    Windsurf,
    VsCodium,
}

impl ExternalEditor {
    pub fn scheme(self) -> &'static str {
        match self {
            ExternalEditor::VsCode => "vscode",
            ExternalEditor::Cursor => "cursor",
            ExternalEditor::Windsurf => "windsurf",
            ExternalEditor::VsCodium => "vscodium",
        }
    }
}

// Encode everything that's not a path-safe character. We preserve `/` as a path separator.
const PATH_SAFE: &AsciiSet = &CONTROLS
    .add(b' ').add(b'"').add(b'#').add(b'?').add(b'<').add(b'>').add(b'`')
    .add(b'{').add(b'}').add(b'|').add(b'\\').add(b'^').add(b'[').add(b']');

pub fn remote_editor_url(editor: ExternalEditor, host_alias: &str, absolute_path: &str) -> String {
    let scheme = editor.scheme();
    let path = utf8_percent_encode(absolute_path, PATH_SAFE).to_string();
    let alias = utf8_percent_encode(host_alias, PATH_SAFE).to_string();
    format!("{scheme}://vscode-remote/ssh-remote+{alias}{path}")
}

pub fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    let cmd = ("xdg-open", vec![url]);
    #[cfg(target_os = "macos")]
    let cmd = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let cmd = ("cmd", vec!["/C", "start", "", url]);
    std::process::Command::new(cmd.0).args(cmd.1).spawn()?;
    Ok(())
}

pub fn host_alias_resolvable_locally(alias: &str) -> bool {
    // Parse ~/.ssh/config. Returns true if alias is a Host entry.
    let Some(home) = dirs::home_dir() else { return false; };
    let config_path = home.join(".ssh/config");
    let Ok(contents) = std::fs::read_to_string(&config_path) else { return false; };
    for line in contents.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Host ") {
            for token in rest.split_whitespace() {
                if token == alias { return true; }
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "external_editor_tests.rs"]
mod tests;
```

Add deps to `app/Cargo.toml`: `percent-encoding = "2"`, `dirs = "5"` (if not already present).

- [ ] **Step 4: Run tests**

Run: `cargo test -p warp external_editor`
Expected: 5 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/external_editor* app/Cargo.toml
git commit -m "Add external_editor URL helper with percent encoding"
```

### Task 2.2: Add `host_alias_resolvable_locally` test

**Files:**
- Modify: `app/src/external_editor_tests.rs`

- [ ] **Step 1: Write a test using a synthetic ~/.ssh/config**

```rust
#[test]
fn alias_resolution_uses_ssh_config() {
    // Write a temp ssh config, point HOME to its dir for the duration of the test.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join(".ssh")).unwrap();
    std::fs::write(tmp.path().join(".ssh/config"), "Host known_alias\n    HostName 1.2.3.4\n").unwrap();
    let old_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", tmp.path());
    assert!(host_alias_resolvable_locally("known_alias"));
    assert!(!host_alias_resolvable_locally("unknown_alias"));
    if let Some(h) = old_home { std::env::set_var("HOME", h); } else { std::env::remove_var("HOME"); }
}
```

Add `tempfile` to dev-deps if missing.

- [ ] **Step 2: Run + commit**

```bash
cargo test -p warp alias_resolution_uses_ssh_config
git add -u
git commit -m "Test host_alias_resolvable_locally with synthetic ssh config"
```

### Task 2.3: Client-side model — RemoteAgentSessionPaneModel skeleton

**Files:**
- Create: `app/src/terminal/remote_agent_sessions/mod.rs`
- Create: `app/src/terminal/remote_agent_sessions/model.rs`
- Modify: `app/src/terminal/mod.rs` (`pub mod remote_agent_sessions;`)

- [ ] **Step 1: Skeleton with state enum**

```rust
// app/src/terminal/remote_agent_sessions/mod.rs
pub mod model;
pub mod resolver;
pub mod registry;
pub mod heartbeat;
```

```rust
// app/src/terminal/remote_agent_sessions/model.rs
use crate::external_editor::ExternalEditor;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct RemoteAgentSessionPaneState {
    pub host_alias: String,
    pub session_id: String,
    pub label: String,
    pub cwd: String,
    pub agent_kind: AgentKindUi,
    pub runtime: RuntimeState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentKindUi { Claude, Codex, Gemini, OpenCode, Custom }

#[derive(Clone, Debug)]
pub enum RuntimeState {
    Attaching { since: Instant },
    Attached { since: Instant, last_offset: u64 },
    Detaching,
    Detached { reason: DetachReasonUi },
    Ended { exit: ExitDescriptorUi },
    Error { detail: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachReasonUi { ClientRequested, Superseded, HeartbeatTimeout, DaemonShutdown }

#[derive(Clone, Copy, Debug)]
pub enum ExitDescriptorUi { Code(i32), Signal(i32) }
```

- [ ] **Step 2: Verify build**

Run: `cargo check -p warp`
Expected: builds clean.

- [ ] **Step 3: Commit**

```bash
git add app/src/terminal/remote_agent_sessions/ app/src/terminal/mod.rs
git commit -m "Scaffold remote_agent_sessions client module"
```

### Task 2.4: Resolver — `resolve_open_action`

**Files:**
- Create: `app/src/terminal/remote_agent_sessions/resolver.rs`
- Create: `app/src/terminal/remote_agent_sessions/resolver_tests.rs`

- [ ] **Step 1: Write tests**

```rust
use super::resolver::*;

#[test]
fn returns_inactive_when_session_id_is_none() {
    let action = resolve_open_action(None, &[], None);
    assert!(matches!(action, OpenAction::Inactive));
}

#[test]
fn returns_attachable_when_session_running_and_no_pane() {
    let summary = DaemonSessionSummary {
        session_id: "s1".into(),
        running: true,
        attached_client: None,
    };
    let action = resolve_open_action(Some("s1"), &[summary], None);
    assert!(matches!(action, OpenAction::Attachable { session_id } if session_id == "s1"));
}

#[test]
fn returns_already_attached_when_pane_exists_for_session() {
    let action = resolve_open_action(Some("s1"), &[], Some("pane-7".into()));
    assert!(matches!(action, OpenAction::AlreadyAttachedToThisClient { pane_id } if pane_id == "pane-7"));
}
```

- [ ] **Step 2: Implement**

```rust
#[derive(Clone, Debug)]
pub struct DaemonSessionSummary {
    pub session_id: String,
    pub running: bool,
    pub attached_client: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenAction {
    Attachable { session_id: String },
    AlreadyAttachedToThisClient { pane_id: String },
    Inactive,
}

pub fn resolve_open_action(
    session_id: Option<&str>,
    daemon_sessions: &[DaemonSessionSummary],
    existing_pane_id_for_session: Option<String>,
) -> OpenAction {
    let Some(id) = session_id else { return OpenAction::Inactive; };
    if let Some(pane_id) = existing_pane_id_for_session {
        return OpenAction::AlreadyAttachedToThisClient { pane_id };
    }
    let matched = daemon_sessions.iter().find(|s| s.session_id == id);
    match matched {
        Some(s) if s.running => OpenAction::Attachable { session_id: id.into() },
        _ => OpenAction::Inactive,
    }
}

#[cfg(test)]
#[path = "resolver_tests.rs"]
mod tests;
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p warp resolver
git add -u
git commit -m "Resolver for remote agent session open actions"
```

### Task 2.5: Registry — multi-view per session with refcount

**Files:**
- Create: `app/src/terminal/remote_agent_sessions/registry.rs`
- Create: `app/src/terminal/remote_agent_sessions/registry_tests.rs`

- [ ] **Step 1: Write tests**

```rust
use super::registry::*;

#[test]
fn register_inserts_view() {
    let mut r = ActiveRemoteAgentViewsModel::default();
    r.register("view-1".into(), ("hostA".into(), "sess-1".into()));
    assert_eq!(r.views_for(("hostA".into(), "sess-1".into())).len(), 1);
}

#[test]
fn unregister_only_emits_close_on_last_view() {
    let mut r = ActiveRemoteAgentViewsModel::default();
    r.register("view-1".into(), ("h".into(), "s".into()));
    r.register("view-2".into(), ("h".into(), "s".into()));
    let evt = r.unregister("view-1");
    assert!(matches!(evt, RegistryEvent::None));
    let evt = r.unregister("view-2");
    assert!(matches!(evt, RegistryEvent::RemoteSessionClosed { host_alias, session_id } if host_alias == "h" && session_id == "s"));
}

#[test]
fn register_evicts_stale_view_for_same_session() {
    let mut r = ActiveRemoteAgentViewsModel::default();
    r.register("view-1".into(), ("h".into(), "s".into()));
    r.register("view-2".into(), ("h".into(), "s".into()));
    // Re-register the same key under a new view: old view-1 should be evicted.
    r.register("view-3".into(), ("h".into(), "s".into()));
    let views = r.views_for(("h".into(), "s".into()));
    assert!(views.contains(&"view-2".to_string()));
    assert!(views.contains(&"view-3".to_string()));
    // After eviction logic of PR #10510: a re-register evicts entries that pointed to the SAME key under DIFFERENT view ids.
    // This test will be tuned to mirror the actual production behavior in the next step.
}
```

(The eviction test will be refined in step 2 based on PR #10510 semantics — re-read that PR's `register_ambient_session` for exact behavior.)

- [ ] **Step 2: Implement**

```rust
use std::collections::HashMap;

pub type ViewId = String;
pub type SessionKey = (String, String); // (host_alias, session_id)

#[derive(Default)]
pub struct ActiveRemoteAgentViewsModel {
    views: HashMap<ViewId, SessionKey>,
}

pub enum RegistryEvent {
    None,
    RemoteSessionClosed { host_alias: String, session_id: String },
}

impl ActiveRemoteAgentViewsModel {
    pub fn register(&mut self, view_id: ViewId, key: SessionKey) {
        // Evict prior view ids that map to the same key but are different view ids.
        // This mirrors PR #10510's retain() behavior.
        self.views.retain(|vid, k| *vid == view_id || *k != key);
        self.views.insert(view_id, key);
    }

    pub fn unregister(&mut self, view_id: &str) -> RegistryEvent {
        let Some(key) = self.views.remove(view_id) else { return RegistryEvent::None; };
        let still_referenced = self.views.values().any(|k| k == &key);
        if still_referenced {
            RegistryEvent::None
        } else {
            RegistryEvent::RemoteSessionClosed {
                host_alias: key.0,
                session_id: key.1,
            }
        }
    }

    pub fn views_for(&self, key: SessionKey) -> Vec<ViewId> {
        self.views.iter().filter(|(_, k)| **k == key).map(|(v, _)| v.clone()).collect()
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p warp registry
git add -u
git commit -m "Registry with refcounted teardown and stale-view eviction"
```

### Task 2.6: Heartbeat task on the client side

**Files:**
- Create: `app/src/terminal/remote_agent_sessions/heartbeat.rs`
- Create: `app/src/terminal/remote_agent_sessions/heartbeat_tests.rs`

- [ ] **Step 1: Implement the loop**

```rust
use std::time::Duration;
use tokio::sync::watch;

pub const HEARTBEAT_INTERVAL_SECS: u64 = 10;

pub struct HeartbeatTask {
    _stop_tx: watch::Sender<bool>,
}

impl HeartbeatTask {
    pub fn spawn<F, Fut>(send_one: F) -> Self
    where
        F: Fn() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (stop_tx, mut stop_rx) = watch::channel(false);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
            loop {
                tokio::select! {
                    _ = interval.tick() => send_one().await,
                    _ = stop_rx.changed() => {
                        if *stop_rx.borrow() { break; }
                    }
                }
            }
        });
        Self { _stop_tx: stop_tx }
    }

    pub fn stop(&self) {
        let _ = self._stop_tx.send(true);
    }
}
```

- [ ] **Step 2: Test the cadence**

```rust
#[tokio::test(start_paused = true)]
async fn heartbeat_calls_send_at_interval() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let _task = HeartbeatTask::spawn(move || {
        let c = c.clone();
        async move { c.fetch_add(1, Ordering::SeqCst); }
    });
    tokio::time::advance(Duration::from_secs(35)).await;
    // Should have fired at 0, 10, 20, 30 = 4 ticks (or 3 depending on first-tick semantics).
    let n = counter.load(Ordering::SeqCst);
    assert!(n >= 3, "expected at least 3 ticks, got {n}");
}
```

- [ ] **Step 3: Run + commit**

```bash
cargo test -p warp heartbeat_calls_send_at_interval
git add app/src/terminal/remote_agent_sessions/heartbeat*
git commit -m "Client-side heartbeat task at 10s cadence"
```

### Task 2.7: Pane view scaffolding

**Files:**
- Create: `app/src/terminal/view/remote_agent_session/mod.rs`
- Create: `app/src/terminal/view/remote_agent_session/header.rs`
- Create: `app/src/terminal/view/remote_agent_session/view.rs`
- Modify: `app/src/terminal/view/mod.rs`

- [ ] **Step 1: Define the view as a `warpui::View`**

Read `warp-ui-guidelines` skill before writing UI — invoke it:

```
Skill: warp-ui-guidelines
```

Then create `view.rs`:

```rust
use crate::external_editor::{remote_editor_url, host_alias_resolvable_locally, ExternalEditor, open_url};
use crate::terminal::remote_agent_sessions::model::*;
use warpui::*;

pub struct RemoteAgentSessionView {
    pub state: RemoteAgentSessionPaneState,
    pub editor: ExternalEditor,
    terminal_view: ViewHandle<crate::terminal::view::TerminalView>,
}

impl RemoteAgentSessionView {
    pub fn new(state: RemoteAgentSessionPaneState, editor: ExternalEditor, ctx: &mut ViewContext<Self>) -> Self {
        // Spin up an inner TerminalView that consumes our byte stream.
        // The exact constructor depends on TerminalView's API — inspect first.
        let terminal_view = ctx.add_typed_action_view(|inner_ctx| {
            crate::terminal::view::TerminalView::new(/* params */)
        });
        Self { state, editor, terminal_view }
    }

    pub fn on_open_in_editor(&self, ctx: &ViewContext<Self>) {
        let url = remote_editor_url(self.editor, &self.state.host_alias, &self.state.cwd);
        let _ = open_url(&url);
    }
}

// Implement View / render in line with warp-ui-guidelines.
```

(The render method follows the team's UI patterns — see the `view/use_agent_footer/mod.rs` and `view/ambient_agent/view_impl.rs` for examples. Header shows label, host, status, and the two action buttons.)

- [ ] **Step 2: Render snapshot test**

If Warp has snapshot tests, write one. Otherwise leave a TODO comment marker — manual smoke covers this.

- [ ] **Step 3: Build**

Run: `cargo check -p warp`
Expected: builds.

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "Remote agent session pane view scaffolding"
```

### Task 2.8: List view — "Sessions on host"

**Files:**
- Create: `app/src/terminal/view/remote_sessions_list/mod.rs`
- Create: `app/src/terminal/view/remote_sessions_list/view.rs`
- Create: `app/src/terminal/view/remote_sessions_list/row.rs`

- [ ] **Step 1: Build the view that calls `ListAgentSessions` and renders rows**

The rows show id-abbreviated, label, agent kind icon, status, last-active, attached-client. Row actions: Attach / Kill / Rename.

Use the patterns from `app/src/ai/agent_management/view.rs` — particularly how table rows + row-action dropdowns are constructed — but stripped down. Reuse `FilterableDropdown` if useful. Do NOT integrate with `AgentConversationsModel`.

- [ ] **Step 2: Auto-refresh every 5 seconds**

Use `tokio::time::interval` inside the view's update loop, or `ctx.spawn_interval` (whatever Warp provides — read existing examples).

- [ ] **Step 3: Build + commit**

```bash
cargo check -p warp
git add -u
git commit -m "Remote sessions list view"
```

### Task 2.9: Byte stream from daemon — `RemoteAgentSessionByteStream`

**Files:**
- Create: `app/src/terminal/remote_agent_sessions/byte_stream.rs`

- [ ] **Step 1: Implement**

```rust
use crate::remote_server::RemoteServerClient;
use tokio::sync::mpsc;

pub struct RemoteAgentSessionByteStream {
    rx: mpsc::Receiver<StreamItem>,
}

pub enum StreamItem {
    Snapshot { bytes: Vec<u8>, truncated_from_start: bool, current_offset: u64 },
    Live { chunk: Vec<u8>, offset: u64 },
    Detached { reason: DetachReason },
    SessionEnded { exit: ExitDescriptor },
}

impl RemoteAgentSessionByteStream {
    pub async fn open(
        client: &RemoteServerClient,
        host_alias: &str,
        session_id: &str,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel(64);
        let session_id = session_id.to_string();
        let client = client.clone();
        tokio::spawn(async move {
            // Send AttachAgentSessionRequest via the existing client RPC layer.
            // Receive AttachAgentSessionEvent push messages, translate to StreamItem,
            // forward through tx.
            // (Exact API depends on RemoteServerClient.)
            let _ = (client, session_id, tx);
        });
        Ok(Self { rx })
    }

    pub async fn next(&mut self) -> Option<StreamItem> {
        self.rx.recv().await
    }
}
```

- [ ] **Step 2: Wire into RemoteAgentSessionView**

When the view is constructed, spawn a task that pulls items from the stream and forwards bytes into the inner `TerminalView`'s ANSI parser.

- [ ] **Step 3: Build + commit**

```bash
cargo check -p warp
git add -u
git commit -m "RemoteAgentSessionByteStream connecting view to daemon attach"
```

### Task 2.10: File explorer remote-root wiring

**Files:**
- Modify: `app/src/code/file_tree/view.rs` (find the `add_remote_root` or equivalent method, line ~896)
- Modify: `app/src/terminal/view/remote_agent_session/view.rs`

- [ ] **Step 1: When pane opens, register host+cwd as a remote root in FileTreeView**

In `view.rs::new`, after constructing, find the workspace's active `FileTreeView` and call its remote-root setter. Use the existing API; do NOT introduce a new one. Inspect lines 896-950 of `file_tree/view.rs` to confirm the method name and signature.

- [ ] **Step 2: Smoke build + commit**

```bash
cargo check -p warp
git add -u
git commit -m "Register remote root in file tree when pane opens"
```

### Task 2.11: Hide Remote Control button on remote agent panes

**Files:**
- Modify: `app/src/terminal/input.rs` (around lines 1104, 2457)

- [ ] **Step 1: Add a condition to skip rendering Remote Control**

Inspect where `StartRemoteControl` action is rendered. Add a guard:

```rust
let is_remote_agent_session = /* check pane kind */;
if !is_remote_agent_session {
    // render Remote Control button as before
}
```

The exact guard depends on how pane kind is reachable from this code path. Add a method like `is_remote_agent_session_pane(view: &TerminalView) -> bool` and call from here.

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "Hide Remote Control button for remote agent session panes"
```

### Task 2.12: Phase 2 verification

- [ ] **Step 1: Run all client-side unit tests**

Run: `cargo nextest run -p warp -E 'test(=remote_agent_sessions::)' -E 'test(=external_editor::)'`
Expected: all pass.

- [ ] **Step 2: Clippy + fmt**

Run: `cargo clippy -p warp --all-targets -- -D warnings && cargo fmt --check`
Expected: clean.

- [ ] **Step 3: Tag**

```bash
git tag phase-2-client-complete
```

---

## Phase 3 — Polish: settings, menu entries, telemetry, manual test docs

### Task 3.1: Settings entry for external editor

**Files:**
- Modify: `crates/settings/src/...` (find the settings schema with `grep -rn "pub struct UserSettings" crates/settings/src/`)

- [ ] **Step 1: Add a key `external_editor: ExternalEditor` with default `VsCode`**

Follow existing settings additions as a template. Settings keys are typically added in `settings_value/src/lib.rs` and `settings/src/...`.

- [ ] **Step 2: Surface in Settings UI**

Add a dropdown in the existing Settings → Editor section.

- [ ] **Step 3: Commit**

```bash
git add -u
git commit -m "External editor preference in settings"
```

### Task 3.2: Menu entries for "New remote agent session…" and "Remote sessions"

**Files:**
- Modify: `app/src/app_menus.rs`

- [ ] **Step 1: Add menu items**

Insert under the "File" or relevant menu, gated by `FeatureFlag::RemoteAgentSessions.is_enabled()`. Dispatch actions:

```rust
AppAction::OpenNewRemoteAgentSessionLauncher,
AppAction::OpenRemoteSessionsListAllHosts,
```

Wire those actions to open the launcher modal and the list view, respectively.

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "Menu entries for remote agent sessions"
```

### Task 3.3: Launcher modal

**Files:**
- Create: `app/src/terminal/remote_agent_sessions/launcher.rs`

- [ ] **Step 1: Build a modal with host picker / agent kind picker / cwd field**

Hosts come from the existing remote hosts model. Agent kind is a fixed enum (Claude/Codex/Gemini/OpenCode/Custom). Cwd starts at the host's `$HOME` (fetched on host selection via an existing RPC or via a small new RPC; if absent, default to `~`).

When the user submits, the modal calls `StartAgentSessionRequest` and then opens a `RemoteAgentSessionView` pane.

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "Launcher modal for remote agent sessions"
```

### Task 3.4: Telemetry events

**Files:**
- Use the `add-telemetry` skill: invoke `Skill: add-telemetry` and follow its guidance to add the 12 events listed in `specs/gh-9416/TECH.md` "Telemetry events" section.

Events:
- `remote_agent_session_started { agent_kind, host_id_hash }`
- `remote_agent_session_attached { agent_kind, host_id_hash, was_running_before, snapshot_bytes_bucket }`
- `remote_agent_session_detached { reason }`
- `remote_agent_session_attach_duration_ms { agent_kind, duration_ms_bucket }`
- `remote_agent_session_kicked`
- `remote_agent_session_killed { agent_kind, duration_bucket }`
- `remote_agent_session_open_in_editor { editor, alias_resolvable }`
- `remote_agent_session_ring_buffer_truncated { session_age_bucket }`
- `remote_agent_session_ended_sessions_evicted { count }`
- `remote_agent_session_heartbeat_timeout` (server-side log only — no client telemetry)
- `remote_agent_session_pty_resize_failure { error }`
- `remote_agent_session_daemon_restart_with_active_sessions { session_count }`

- [ ] **Step 1: For each event, follow add-telemetry**

Emit at: pane open (`_started`), attach success (`_attached`), pane close (`_detached`, with reason), session pane lifetime end (`_attach_duration_ms`), kicked-event arrives (`_kicked`), KillAgentSession success (`_killed`), Open-in-editor click (`_open_in_editor`), inspect snapshot shows truncations > 0 (`_ring_buffer_truncated` — fire from daemon via inspect, or surface via the snapshot truncated_from_start), eviction count reported by inspect (`_ended_sessions_evicted`), PTY resize failure (`_pty_resize_failure`), on reconnect-after-daemon-restart if previous session count was > 0 (`_daemon_restart_with_active_sessions`).

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "Add 11 telemetry events for remote agent sessions"
```

### Task 3.5: Manual test script

**Files:**
- Create: `script/manual-tests/remote-agent-sessions.md`

- [ ] **Step 1: Document the manual scenarios**

```markdown
# Manual test: persistent remote CLI agent sessions

## Setup
1. Configure an SSH host with Warp (one you have actual SSH access to).
2. Enable `FeatureFlag::RemoteAgentSessions` (dogfood builds have this on by default).
3. Bootstrap `remote_server` on the host via the usual SSH flow.

## Scenario 1 — Survive client close
1. Menu → New remote agent session → choose host, Claude Code, cwd = some real project on the host.
2. Submit a prompt to the agent.
3. While the agent is working, close the Warp window completely.
4. Wait 1 minute.
5. Reopen Warp. Menu → Remote sessions. Find the session, click Attach.
6. Verify: terminal renders the conversation that happened while you were away. Tool-use banners still appear. Approval prompts work.

## Scenario 2 — Move between machines
1. Same as Scenario 1 steps 1–2, but on Laptop A.
2. SSH host alias is in Laptop A's `~/.ssh/config`. Laptop B has the same alias.
3. On Laptop B, open Warp, configure the same SSH host (same alias).
4. Menu → Remote sessions on `<host>`. Click Attach on the session.
5. Verify: Laptop A's window shows banner "Session attached from another device" and closes after 2s.
6. Laptop B continues the session smoothly.

## Scenario 3 — Open in editor with spaces in path
1. Start a session with `cwd = ~/projects/my project` (with space).
2. Click "Open in editor".
3. Verify VSCode opens with Remote-SSH at the right path.

## Scenario 4 — Open in editor from a machine without local SSH alias
1. From Laptop B above: click Open in editor.
2. Verify tooltip warning appears: "This host alias is not configured locally..."
3. Click "Copy alias", verify clipboard contains the alias.

## Scenario 5 — Kill in progress
1. Run an agent that's actively producing output.
2. Click "End session".
3. Verify pane shows "Session ended" banner.
4. Click Attach from list view: read-only mode confirmed (input rejected).
```

- [ ] **Step 2: Commit**

```bash
git add script/manual-tests/remote-agent-sessions.md
git commit -m "Manual test script for remote agent sessions"
```

### Task 3.6: Phase 3 verification

- [ ] **Step 1: Full presubmit**

Run: `./script/presubmit`
Expected: passes.

- [ ] **Step 2: Tag**

```bash
git tag phase-3-polish-complete
```

---

## Phase 4 — Integration tests & coordination

### Task 4.1: Integration test — persistence across client disconnect

**Files:**
- Create: `crates/integration/tests/remote_agent_sessions.rs`

- [ ] **Step 1: Write the test using Warp's integration harness**

Read `warp-integration-test` skill before writing.

The test spins up a real `remote_server` daemon (in-process or via the harness), starts a session running `bash -c 'while :; do echo tick; sleep 1; done'`, attaches, drops the SSH transport mid-stream, reconnects with a different client identity, attaches again, verifies the snapshot contains "tick" lines.

- [ ] **Step 2: Run**

Run: `cargo nextest run -p integration remote_agent_sessions`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/integration/tests/remote_agent_sessions.rs
git commit -m "Integration test: persistence across client disconnect"
```

### Task 4.2: Integration test — kick from second client

**Files:**
- Modify: `crates/integration/tests/remote_agent_sessions.rs`

- [ ] **Step 1: Test that a second client with a different identity attaching to the same session causes the first to receive `Detached { Superseded }`**

- [ ] **Step 2: Run + commit**

```bash
cargo nextest run -p integration remote_agent_sessions::kick
git add -u
git commit -m "Integration test: kick semantics"
```

### Task 4.3: Integration test — heartbeat timeout in partition

**Files:**
- Modify: `crates/integration/tests/remote_agent_sessions.rs`

- [ ] **Step 1: Test that withholding heartbeats for >25s while keeping the transport open causes the daemon to release the lock with `HeartbeatTimeout` and allows a second client to attach**

- [ ] **Step 2: Run + commit**

```bash
cargo nextest run -p integration remote_agent_sessions::heartbeat
git add -u
git commit -m "Integration test: heartbeat timeout releases lock in partition"
```

### Task 4.4: Coordination comment on #9416

**Files:**
- Create: `docs/superpowers/notes/2026-05-17-gh-9416-comment-draft.md`

- [ ] **Step 1: Draft the comment (do NOT post yet)**

```markdown
# Draft comment to post on issue #9416

Hey @kevinyang372 / @petradonka — wrote up a spec for the SSH half of the
"Persistent sessions locally and over ssh, pane detaching" line from
roadmap #9233. It's at `specs/gh-9416/` in PR <pending>. Summary:

- Extend `remote_server` daemon with an `AgentSessionManager` holding
  `(child, pty, ring_buffer, attach_lock)` per session.
- 9 new proto messages on the existing protocol (Start/List/Attach/
  Detach/Heartbeat/Kill/Rename/Input/Resize + Inspect).
- Client adds a new pane type "Remote Agent Session", a sessions list view,
  and a resolver/registry mirroring patterns from PR #11097 / #10426 / #10510
  (Zach Bai's cloud-agent re-entry series).
- Gated behind `FeatureFlag::RemoteAgentSessions`. Reuses the existing
  cli_agent_sessions observation, terminal model, file-tree (already
  remote-aware), voice input, etc.
- v1 scope: SSH hosts only. v2 path: HTTP/WS transport on the same daemon
  for a PWA — `RemoteServerManager`'s transport-agnostic seam makes it additive.
- Local session persistence (the other half of the roadmap line) is left
  as an obvious follow-up using the same `AgentSessionManager` pattern.

Wanted to surface before opening the implementation PRs in case there's
unpublished work in the same area. Happy to iterate on the spec.
```

- [ ] **Step 2: Commit the draft**

```bash
git add docs/superpowers/notes/
git commit -m "Draft #9416 coordination comment"
```

- [ ] **Step 3: Post manually**

This step is intentionally manual. Wait for human review before posting.

### Task 4.5: Final presubmit + PR

- [ ] **Step 1: Run full presubmit**

Run: `./script/presubmit`
Expected: green.

- [ ] **Step 2: Open PR with this spec as context**

Use the PR template at `.github/pull_request_template.md`. Link the spec at `specs/gh-9416/`. Include:
- `CHANGELOG-NEW-FEATURE: Remote agent sessions persist across Warp restarts when running on a remote SSH host.`
- Link to issue #9416 with `Closes #9416` (partial — local persistence is follow-up).

- [ ] **Step 3: Tag final**

```bash
git tag phase-4-integration-complete
```

---

## Self-review notes

This plan covers all 27 PRODUCT.md behaviors via the test mapping in TECH.md, plus the architecture fixes from the review (heartbeat, LRU, ANSI-boundary trim, hard reset prepend, URL encoding, alias warning, telemetry enumeration, inspect RPC). Each task is a TDD cycle that fits the team's existing patterns and convention (skills: `add-feature-flag`, `add-telemetry`, `warp-ui-guidelines`, `warp-integration-test`). Implementer should read each referenced skill at the start of the corresponding task — that is the local convention.
