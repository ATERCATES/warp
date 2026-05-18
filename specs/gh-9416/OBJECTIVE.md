# Persistent Remote CLI Agent Sessions — Brainstorming notes

Companion to `PRODUCT.md` and `TECH.md` in this directory. Captures the journey to the design — why we picked what we picked, what we ruled out. The canonical spec lives in PRODUCT.md (behavior) and TECH.md (implementation). This file is for context, not for implementation.

## What the user wants

CLI agents like Claude Code, Codex, Gemini CLI, OpenCode running on a remote SSH host should keep running when the desktop Warp client closes, sleeps, or restarts. The user should be able to re-attach from any machine and continue. Eventually a mobile PWA should also be able to observe these sessions (deferred to v2).

## How the problem reframes once you look at the code

Most of the apparent complexity evaporates after one pass through the codebase:

- **The "agent" is not Warp's code.** It is an external CLI process. Its conversation persistence is the CLI's responsibility (Claude writes to `~/.claude/projects/…` itself; Codex/Gemini analogous). The only thing that dies when Warp closes today is the *process and its PTY*.
- **The PTY-over-network plumbing already exists.** `crates/remote_server` does it. `RemoteServerController` wires a remote PTY to the terminal model. The model and ANSI parser are agnostic to byte source.
- **The CLI agent observation pipeline (OSC 9277 / OSC 9) is byte-driven.** `cli_agent_sessions/listener` subscribes to events that originate from any byte stream that flows through the ANSI parser. It does not know or care if the PTY is local or remote.
- **The transport abstraction is already there.** `RemoteTransport` (`crates/remote_server/src/transport.rs:184`) is object-safe and documented as "transport-agnostic session lifecycle managed by `RemoteServerManager`. Alternative transports (Docker exec, in-process for tests) implement the same trait without touching the manager." This is the seam v2 PWA will use.
- **The team already framed the gap.** The `oz-for-oss[bot]` on [issue #9416](https://github.com/warpdotdev/warp/issues/9416) concluded: *"A durable PTY/session owner, or a narrower update-handoff helper, would address the primary update-survival pain more directly than special-casing agent sessions."* That durable PTY/session owner — for the remote case — is what this spec proposes.

What remains to build: a `AgentSessionManager` inside the existing `remote_server` daemon that holds `(child, pty, ring_buffer, attach_lock)` per session, plus 8 new proto messages for start/list/attach/detach/kill/rename/input/resize, plus client-side UI for creating panes and listing sessions. The byte buffer is in-memory only — CLI agents persist their own conversations.

## Options considered

**A — Tmux as the persistence engine.** Spawn each agent as a tmux session, use tmux attach/detach. **Rejected.** Tmux strips unknown OSC sequences by default; passing through requires explicit DCS wrapping by the inner application, which Claude/Codex/Gemini/OpenCode do not do. The OSC 9277 / OSC 9 observation pipeline would silently break. Implementing `tmux -CC` (control mode) parsing in Warp to fix this is more work than just building our own minimal session owner. (Tangentially confirmed by Warp docs: legacy tmux Warpify integration is being deprecated.)

**B — New HTTPS/gRPC daemon, parallel to `remote_server`.** Build a separate daemon with its own protocol, auth, install flow. **Rejected for v1.** Duplicates work the team is actively doing (~30 PRs/10 days on `remote_server`). v2 PWA naturally adds an HTTP/WS transport to the same daemon via the existing `RemoteTransport` abstraction — no parallel binary needed.

**C — `dtach` / `abduco` + our own scrollback.** Minimal detach utilities pipe bytes transparently without parsing, so they preserve OSC. **Rejected.** They have no scrollback, which is exactly the part we would otherwise have for free in option A or build ourselves in option D. We would end up writing the ring buffer anyway, plus an external dependency.

**D — Extend `remote_server` with a session manager (selected).** Add `AgentSessionManager` inside the existing daemon, expose 8 new RPCs on the existing protocol, add a new pane type on the client. Reuses the SSH transport, install flow, reconnect logic, auth context, and entire byte pipeline. Smallest delta, no external deps, no UI degradation. See `TECH.md`.

## Positioning against existing Warp features

- **Remote Control** (`/remote-control`) publishes a running CLI agent to Warp's cloud for mobile/web monitoring. It does **not** survive Warp client restart on the host side, and the runtime is the user's local machine.
- **Cloud Agents / Oz** runs persistent agents in Warp-managed VMs (or self-hosted Docker/K8s). It does **not** target the user's own SSH host.
- **Ambient Agents** are background agents on the Warp Platform.
- **This spec** fills the explicit roadmap line "Persistent sessions locally and over ssh, pane detaching" ([#9233](https://github.com/warpdotdev/warp/issues/9233), May–June 2026) for the SSH half. Local persistence is the natural follow-up and matches [issue #9416](https://github.com/warpdotdev/warp/issues/9416)'s framing exactly.

## Decisions locked during brainstorming

These are settled and reflected in PRODUCT.md / TECH.md. Listed here for traceability:

- Persistence: in-memory ring buffer (~4 MiB), no on-disk log.
- Attach policy: single active client, kick on second attach (`tmux attach -d` semantics).
- Naming: `<agent>: <cwd>` default, editable.
- Env vars: daemon-only, no per-session overrides in v1.
- Editors: VSCode, Cursor, Windsurf, VSCodium (JetBrains deferred).
- Hosts: Linux and macOS (Windows deferred).
- Lifecycle on agent exit: session stays `Ended` in daemon memory until explicit kill or daemon restart.
- Lifecycle on daemon restart: all sessions die.

## Coordination before landing code

Open a comment on [#9416](https://github.com/warpdotdev/warp/issues/9416) with the PRODUCT.md bullet summary, tagging `@kevinyang372` (leads daemon/transport on `remote_server`) and `@petradonka` (milestone manager). Wait for ack or 5 business days before the Prelude PR. The "Persistent sessions over ssh" roadmap line has no owner, no in-flight PRs, and only the oz-bot's analysis on #9416 has touched the topic — but `remote_server` is heavily-developed territory and a courtesy ping prevents collision with unpublished plans.
