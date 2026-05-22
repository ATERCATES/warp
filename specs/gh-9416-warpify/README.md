# Warpify Remote Tmux Sessions (gh-9416 follow-up)

## Goal

Make tabs opened from the Remote Sessions panel behave as first-class
`SessionType::WarpifiedRemote { host_id: Some(_) }` so every existing and
future Warp feature that already handles remote sessions (Open in IDE,
AI file tools, code review, repo detection, conversation handoff, ...)
works against them with **zero per-feature plumbing** in our fork.

## Non-goals

- Rewriting the warpify SSH detection flow. We reuse it.
- Replacing the tmux `-CC __warp_ctrl` control plane. It stays.
- Supporting attach-to-arbitrary-preexisting-tmux-sessions with full UI
  parity. Sessions created by Warp are first-class; legacy sessions are
  best-effort.

## Architecture

```
+-- Host (remote) ------------------------------+
|                                               |
|  +--------------------+  +------------------+ |
|  | warp-remote-server |  | tmux server      | |
|  | daemon (RPC ops)   |  |  +- user-sess-A  | |
|  |                    |  |  +- user-sess-B  | |
|  +--------------------+  |  +- __warp_ctrl  | |
|                          +------------------+ |
+-----------+------------------+----------------+
            ^                  ^
            | ssh RPC          | ssh -CC
            | (SshTransport)   | (panel control plane)
            |                  |
+-----------+------------------+----------------+
| Warp client                                   |
|  +- RemoteServerManager (host_id mapping)     |
|  +- RemoteSessionsModel  (panel state)        |
|  +- RemoteAttachRegistry (session->host map)  |
|  +- Tab(WarpifiedRemote{host_id:Some(.)})     |
+-----------------------------------------------+
```

Two control planes coexist over the same SSH ControlMaster:

- `warp-remote-server` daemon handles all "is the filesystem remote?"
  features (file IO, git, repo indexing, AI tools).
- `tmux -CC __warp_ctrl` handles panel-specific UX (list/create/kill
  tmux sessions, agent CLI status, attach).

## Implementation phases

### Phase 1 (this branch) - Remote-attach context registry

A new singleton `RemoteAttachRegistry` keyed by `SessionId` holds
`(local_host_key, session_name)` for each tab opened from the panel.
This is consulted by the few site-specific decisions that today
hard-code "local session = local filesystem":

- `TerminalView::open_working_dir_in_editor` -> when the session is
  registered, route to the SSH editor path with the host from settings.
- (Later phases) `pwd_as_local_or_remote`, AI file tools, code review.

Tradeoffs:
- + Zero changes to the `Session` / `SessionType` model.
- + Cfg-gated entirely; non-`remote_sessions` builds compile out the
  registry and the lookup branches.
- + Reversible without touching upstream code.
- - Each feature that needs awareness still requires a tiny patch.
  Acceptable as a first step; Phase 2 removes this.

### Phase 2 - Full WarpifiedRemote integration via RemoteServerManager

Promote remote-attach tabs to `SessionType::WarpifiedRemote { host_id }`
by hooking into `RemoteServerManager`:

1. On `RemoteSessionsModel::connect`, after the ControlMaster comes up,
   build an `SshTransport` reusing our socket path and `auth_context`,
   then call `RemoteServerManager::connect_session(...)`. Listen for
   `SessionConnected { host_id }` and store it on the panel state.
2. When opening a remote-attach tab, plumb the bootstrap session type
   to `BootstrapSessionType::WarpifiedRemote` and the host_id to the
   newly created `Session` via `session.set_remote_host_id(...)`.
3. The shell that runs inside the attached tmux receives the bootstrap
   script via PTY; its `InitShell` payload carries the remote hostname,
   so `determine_session_type` confirms `WarpifiedRemote` naturally.
4. Once `host_id` is present, every existing feature that already
   branches on `WarpifiedRemote { host_id: Some(_) }` works without a
   per-feature patch.

Risks documented under `risks.md` (TODO):
- Bootstrap path assumes a shell command; we run `tmux attach`. Mitigate
  by sending the bootstrap script via PTY after the attach lands inside
  the remote shell (which is what the existing SSHTmuxWrapper flow does).
- Daemon and our control plane share the same ControlMaster socket but
  open separate logical channels. Verify no contention on slow links.
- `SshRemoteServer` feature flag is in RELEASE; `SSHTmuxWrapper` is
  DOGFOOD-only. We depend on `SshRemoteServer`. If the legacy daemon
  flow is retired, follow that.

### Phase 3 - UX for legacy sessions

Tmux sessions that existed before Warp connected won't have a
warpified shell. Mark them in the panel as "non-warpified" and either
skip the bootstrap injection or offer a manual "warpify now" action.
Default behaviour: only allow attach to Warp-created sessions; show
others as read-only with a hint.

## Upstream-tracking contract

To keep the fork mergeable with `warpdotdev/warp` long-term:

1. **Feature gate everything**: `#[cfg(feature = "remote_sessions")]`
   wraps every panel-related codepath, registry entry, action variant,
   and call site. Builds without the feature produce zero panel code.
2. **Reuse upstream abstractions**: `BootstrapSessionType`,
   `SessionType`, `WarpifyState`, `RemoteServerManager`, `SshTransport`.
   Do not invent parallel concepts.
3. **Minimal touch points in upstream files**: each insertion is either
   a small `cfg`-gated match arm or a delegation to a method in our
   own module.
4. **Watch points** (functions whose signature changes break us):
   - `SessionType::WarpifiedRemote` enum shape
   - `RemoteServerManager::connect_session` signature
   - `WarpifyState::set_pending_ssh_host` signature
   - `build_remote_attach_tab_config` (this file is ours)
   - `Workspace::OpenRemoteAttachTab` handler
5. **Smoke test contract**: an integration test that wires a panel-spawned
   session next to a user-typed `ssh host` session and asserts equivalent
   `SessionType`, `host_id`, `pwd_as_local_or_remote`, `active_ssh_host`.
   Breaks loudly if upstream changes the contract.

## File layout

```
app/src/terminal/remote_sessions/
+-- mod.rs
+-- attach_registry.rs   <-- Phase 1: SessionId -> (host_key, session_name)
+-- connection.rs        (existing) tmux -CC __warp_ctrl
+-- model.rs             (existing) panel state
+-- commands.rs          (existing) tmux command builders
+-- probe.rs             (existing) host capability detection
+-- types.rs             (existing) shared types
+-- remote_server_bridge.rs  <-- Phase 2 (planned)

app/src/workspace/view/remote_sessions_panel/  (existing UI)
specs/gh-9416-warpify/   <-- this design doc
```

## Open questions

- Does the existing bootstrap script handle being sent to a shell that
  is already running inside an attached tmux session? Probably yes (the
  SSHTmuxWrapper does effectively this), but to verify before Phase 2.
- Can `RemoteServerManager` operate against a shell that started as
  `tmux attach` rather than a fresh `ssh ... bash --login`? The daemon
  is shell-agnostic; the bootstrap timing is the risk.
- For sessions created by the panel `+` button, we control the spawn
  and can inject `WARP_BOOTSTRAP=1` env vars cleanly. For attach-to-
  preexisting, the shell is already alive and bootstrapping it requires
  sending the bootstrap inline at attach-time. Open whether to do it
  unconditionally or behind a per-host setting.
