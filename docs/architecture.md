# How the pieces fit

Switchboard is four programs. This page says what each one does and how they talk, so you know where to look when
you change something.

```
  app (sb-desktop)  --HTTP :8130-->  server (server/)   --ssh, or through the hub-->  helper (agent/sb.py)
        |                                 |                                               ^
        +--------WebSocket :8132--------> hub (sb-hub) <----WebSocket, outgoing---- switchboard-node
```

## The server: `server/`

Python, standard library only, one process. It runs on the computer that stays on.

- Every few seconds it asks each computer's helper what is running and keeps the answer as one snapshot.
- It serves the HTTP API on port 8130 (`/api/sessions`, `/api/stream` for live updates, and the actions: start,
  resume, stop, continue on another computer). The apps only ever talk to this API and the hub.
- It keeps settings in `~/.config/switchboard/` and history in `~/.local/share/switchboard/`.
- Port 8131 serves the files a new computer downloads when it is added.

Module map: `server.py` (HTTP routes, the poller), `fleet.py` (calls to the computers), `nodes.py` (computers linked
through the hub), `term.py` (terminal tickets), `features.py` (cleanup, threads, picking a computer, dispatch),
`budget.py` (the usage budget), `library.py` (tools shared between computers), `flags.py`, `notes.py`,
`bonjour.py` (finding the server on the local network).

## The helper: `agent/sb.py`

One Python file, copied to `~/.claude/switchboard/sb.py` on each computer. It is Claude Code's status line and
hooks there, so it knows each chat's state as it changes, and it answers the server's requests (list chats, start
one, stop one). The server reaches it in one of two ways:

- **ssh**, with a key that can only run `sb.py` (a forced command, no shell);
- **a link through the hub**, for computers that have no ssh server or sit behind another router.

## The hub: `native/crates/sb-hub`

Rust, runs next to the server on port 8132. It carries terminals: the app asks the server for a ticket
(`POST /api/term`), then opens a WebSocket to the hub with it, and the hub connects that to the chat's terminal on
its computer. It also holds the links from `switchboard-node` and the pairing codes for adding a computer.

## The node: `native/crates/sb-node`

A small Rust program on computers added with a code. It keeps one outgoing WebSocket open to the hub, so the
service can reach that computer without ssh.

## The app: `native/crates/sb-desktop` and `sb-core`

`sb-core` has the models, the rules for which column a chat belongs in, the HTTP client and the snapshot cache. It
has no UI and most of the tests. `sb-desktop` is the app, built with GPUI.

The app never waits on the network to draw: it shows the last snapshot it saved, then replaces it when the
service answers, and refreshes in the background.

## Ports

| Port | What |
|---|---|
| 8130 | The server's API |
| 8131 | Downloads for adding a computer |
| 8132 | The hub: terminals and linked computers |

None of them has a password. See [SECURITY.md](../SECURITY.md).
