# Persistent Remote CLI Agent Sessions — Product Spec

Linear: none. GitHub: [#9416](https://github.com/warpdotdev/warp/issues/9416) (relates), [#10185](https://github.com/warpdotdev/warp/issues/10185) (relates), [#9233](https://github.com/warpdotdev/warp/issues/9233) (roadmap line). Figma: none provided.

## Summary

Warp gains a new pane type, "Remote Agent Session", that runs a CLI agent (Claude Code, Codex, Gemini CLI, OpenCode, or a user-supplied command) on a remote SSH host through the existing `remote_server` daemon. The agent's process and PTY live on the host, so closing the Warp client, restarting it, or moving to another machine no longer kills the session. The user can list and re-attach to live sessions on each configured host, with kick semantics for the single-active-client model. Each session pane has a one-click "Open in editor" action that opens VSCode (or another configured editor) over Remote-SSH at the same host and working directory.

## Problem

A CLI agent launched inside a Warp SSH tab today is anchored to the desktop client: when the client closes or crashes, the chain of resources holding the SSH process collapses and the daemon tears the agent's PTY and process down. Consequence: closing the laptop, restarting Warp, or moving to a different machine wipes any in-flight agent work that has not already been persisted by the agent itself.

The roadmap line *"Persistent sessions locally and over ssh, pane detaching"* commits to filling this gap. There is no owner yet, and the team's own automated analysis already framed the missing piece as *"a durable PTY/session owner."* This spec is that durable session owner, scoped to remote SSH hosts as v1.

## Goals

- A CLI agent process and its PTY survive a desktop client close, crash, autoupdate, OS sleep, or network drop on the client side.
- A user can re-attach to a running session from the same machine after relaunching Warp, or from a different machine that has SSH access to the same host.
- The session catalogue is queryable per host: the user can see what is running, when it was last active, and which agent kind it is.
- The user can open the session's working directory in their local editor (VSCode by default) connected to the remote host via Remote-SSH.

## Non-goals

- PWA, mobile, or web client. Deferred to a v2 on the same architecture (additive HTTP/WS transport).
- Multi-client mirror (a second client observing while the first controls).
- A persistent on-disk log of session output. The in-memory buffer used for re-attach replay is sufficient; CLI agents persist their own conversation state.
- Surviving a daemon restart, host reboot, or remote_server autoupdate. When the daemon goes down, its sessions go down with it.
- Migrating local sessions to remote or vice versa. A session is bound to its host for life.
- Running Warp's native agent on the remote daemon. This feature is exclusively for external CLI agents.
- Per-session environment variable overrides from the client. v1 uses the daemon's env only.
- JetBrains Gateway as an "Open in editor" target. VSCode, Cursor, Windsurf, VSCodium only.
- Windows hosts. v1 targets Linux and macOS hosts.

## Behavior

### Creating a session

1. A new menu entry "New remote agent session…" is reachable from the same surface as "New SSH tab" and from the host's context menu in the existing remote hosts UI. The entry is only present when the feature is enabled locally AND the daemon on the chosen host has indicated support for remote agent sessions.

2. The launcher prompts for: (a) host (defaults to the most-recently-used remote host), (b) agent kind (`Claude Code` / `Codex` / `Gemini CLI` / `OpenCode` / `Custom command`), (c) working directory on the remote host (absolute path; defaults to the host user's `$HOME`). When `Custom command` is selected, the launcher additionally prompts for a command string and an optional argument list.

3. On confirmation, a new pane opens immediately in an "Attaching…" state. The pane header shows the agent kind, host alias, and the working directory the user just chose; the body shows a loading indicator. Within ~1–2 seconds (typical SSH RTT) the pane transitions to "Attached" and the user can interact with the agent. If the daemon takes longer than 10 seconds to confirm the session is up, the loading indicator becomes an explicit progress message naming the step in progress.

4. If the daemon rejects the start (binary not found, cwd invalid, OS error), the pane shows an error banner with the daemon's reason and offers two actions: "Try a different command" (reopens the launcher pre-filled with the previous values) and "Close". The pane does not auto-close on error.

### Interacting with a session

5. The pane body is a terminal model identical to any other terminal pane: the user types, the bytes go to the remote PTY; the PTY's bytes come back and render in the model. The agent's tool-use banners, approval prompts, and "Use agent" affordances surface exactly as for a local agent of the same kind — the user cannot tell the agent is remote except through the host label in the header.

6. The pane header shows: the editable session label (default `<agent>: <cwd>`, e.g. `claude: ~/projects/warp`), the host alias, the agent kind icon, the session status (`Attaching` / `Attached` / `Detached` / `Ended` / `Error`), an "Open in editor" button, and an "End session" button. The label is editable in-place at any time. Renaming propagates to the daemon so other clients see the new label in the list view.

7. **Open question:** the "Open in editor" button uses the *initial* `cwd` recorded at session start, not whatever directory the agent may have navigated into internally. Detecting current cwd inside the agent's own process is out of scope. Confirm "initial cwd" is acceptable; otherwise, the only way to make it "current" is for the user to manually update the label/cwd, which v1 does not expose.

8. Closing the pane (`Cmd/Ctrl+W`, close button) **detaches** the session: the client transitions through "Detaching" and discards the pane. The agent process continues running on the host. There is no implicit "auto-kill on close".

9. Clicking "End session" **kills** the session: the daemon sends SIGTERM to the agent (escalating to SIGKILL after a 5-second grace period), closes the PTY, and transitions the session to `Ended`. The pane shows a brief banner "Session ended" and closes after 2 seconds.

10. Resizing the pane (window resize, sidebar open/close, pane split) propagates the new terminal dimensions to the agent on the host, which sees a window-change signal as it would on a local terminal.

11. The attached client sends a heartbeat to the daemon on a fixed cadence (roughly every 10 seconds) while attached. If the daemon misses two consecutive heartbeats, it considers the client gone and releases the attach lock so another client can attach. This is the only signal of liveness the daemon trusts; the underlying transport's disconnect events are advisory hints, not authoritative releases.

### Listing and re-attaching from elsewhere

12. A new view "Sessions on `<host>`" is reachable from the host's context menu and from a new top-level menu "Remote sessions" that aggregates across all configured hosts. Each row shows: abbreviated id (first 8 chars), label, agent kind icon, status, last-active timestamp, and the label of the currently-attached client (if any). Row actions: "Attach", "Kill", "Rename".

13. Clicking "Attach" on a session that is `Running` and has no attached client opens a fresh pane in "Attaching" state. The daemon sends a snapshot of the session's recent output, followed by live output. The terminal model processes the snapshot to reconstruct the visible screen state — including scrollback — before live output starts flowing.

14. Clicking "Attach" on a session that already has an attached client kicks the previous attach. The previous client receives a typed event causing its pane to display a banner "Session attached from another device" and close cleanly after 2 seconds. The new client begins receiving snapshot + live output immediately. No confirmation is shown — the kick is part of the documented model.

15. **Open question:** when the new client kicks the previous one and there is unsent input pending in the previous client's local buffer (e.g., a half-typed line), that input is lost on the previous client's side. We do not surface this loss explicitly. Decide whether a "stuck-input rescue" toast is worth shipping in v1 or if it can wait.

16. Clicking "Attach" on a session in `Ended` state opens a read-only pane: the user sees the final state of the output, but typed input is rejected with a footer notice "This session has ended". The pane has no "End session" button (the session is already ended); only a close button.

17. If the daemon truncated the snapshot because the agent produced more output than the snapshot holds, the user sees a single synthetic line at the top of the scrollback reading `(earlier output truncated)`. The screen state and any output following the truncation render correctly — the truncation never corrupts subsequent rendering.

18. The list view auto-refreshes every 5 seconds while the user is viewing it. The user can also manually refresh. Eventually-consistent: a session that was just killed may briefly appear in the list with `Ended` status before being removed.

### Editor integration

19. The "Open in editor" button opens the user's configured editor over Remote-SSH at the session's host and `cwd`. The default editor is VSCode. Users can change the default in Settings → "External editor" to Cursor, Windsurf, or VSCodium. Paths with spaces, unicode, or shell metacharacters open correctly.

20. The editor opens connected to the same SSH host the session is running on, at the same working directory. When the host alias is not present in the local `~/.ssh/config` of the machine running Warp (e.g., the user attached from a different laptop than the one that originally configured the host), the button shows a warning tooltip "This host alias is not configured locally; the editor will not be able to connect" and a "Copy alias" action so the user can add it to their config. The user can still click the button; the editor will surface its own error.

21. The button is enabled whenever the session has a known `cwd` (always true for v1 sessions). It is disabled in read-only `Ended` sessions only if the configured editor scheme is unavailable on the local platform.

### Persistence boundaries

22. Sessions persist in the daemon's memory across desktop-client disconnects, OS sleeps, and Warp restarts. They do NOT persist across daemon restarts, host reboots, or remote_server autoupdates: in those cases, all sessions terminate when their child processes die. When the user reconnects from the desktop after a daemon restart, the list view is empty, and a one-time banner explains "Sessions were terminated by daemon restart" if the user had a session attached at the moment of the restart.

23. When the agent process exits on its own (graceful exit, crash, signal), the session transitions to `Ended` and remains visible in the list view. Ended sessions are bounded: the daemon keeps at most a small fixed number of recent `Ended` sessions per host (in the order of dozens); when that cap is exceeded, the oldest `Ended` session is evicted. Re-attaching to an `Ended` session is read-only (per behavior 16). Running sessions are never evicted.

24. The desktop client does not cache any per-session state locally beyond the user's currently-open pane's state (which is discarded on close) and the editable label (which is propagated to the daemon and not stored client-side). The list view is fetched on demand.

### Feature gating and compatibility

25. The entire feature is gated behind a feature flag with default off (on for dogfood builds). When the flag is off, the new menu entries, view, and pane type are absent. The daemon-side support is unconditional once shipped; the client decides whether to use it.

26. The daemon advertises support during the existing session-bootstrap handshake. Hosts running an older daemon (no advertisement) show their existing UI only; the host card's tooltip explains that this host needs to be upgraded.

27. Local agent sessions, the native Warp agent, and all existing pane types are unchanged by this feature.

## Open questions

These are flagged inline in the relevant behaviors above (7, 15). Both can be resolved during implementation without changing the architecture.
