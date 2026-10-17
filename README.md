# Switchboard

Switchboard puts every Claude Code chat from all of your computers on one board. You can see which chats need
you, answer them, open their terminals, start new ones on whichever computer you pick, and move a chat from one
computer to another.

![The Switchboard board on a Mac](docs/screenshots/board-dark.png)

## How it works

Switchboard has three pieces. You only have to think about the first one once.

```
   the computer that stays on                 every computer you use Claude Code on
  +----------------------------+             +--------------------------------------+
  |  Switchboard server        | <---------> |  Switchboard helper (sb.py)          |
  |  keeps the list of chats   |             |  reports its chats, starts and stops |
  +----------------------------+             +--------------------------------------+
               ^
               |   your home network, or Tailscale when you are away
               v
  +----------------------------+
  |  Switchboard app           |
  |  Mac and Windows           |
  +----------------------------+
```

1. **One computer that stays on.** It keeps track of your chats and your computers, so the board is there even
   when your laptop is closed. A Mac mini on a shelf, a desktop PC that never sleeps, or a small Linux box all
   work. If you only have one computer, that computer can do this job too.
2. **A small helper on each computer where you use Claude Code.** It tells the server what is running and does
   what you ask from the board: start a chat, stop one, open its terminal.
3. **The app.** It shows the board, on a Mac or a Windows PC. Phone apps are on the way. It talks only to your own server, never to anything on the internet.

Nothing leaves your computers. Switchboard has no account, sends no usage data, and does not need a cloud
service.

## What you need

- A Mac or Linux computer that can stay on. Windows works too, with fewer features (see the install guide).
- Python 3.9 or newer on every computer.
- Claude Code installed and signed in on the computers where you want to run chats.

## Install

1. **On the computer that stays on**, get Switchboard and run the installer:

   ```sh
   git clone https://github.com/nicglazkov/switchboard.git
   cd switchboard
   scripts/install/install-server.sh
   ```

   On Windows, run `scripts\install\install-server.ps1` in PowerShell instead. The installer needs no admin
   rights and changes nothing outside your own account.

2. **Answer three questions.** The installer asks for the address your other devices will use, which computers
   to add, and the folder where new chats start on each one. Run `switchboard setup` to change them later.

3. **Add your other computers.** Setup shows one line to paste on each. It installs the helper and links the
   computer with a one-time code, so there are no passwords or ssh keys to manage.

4. **Open the app** and type the address from step 2, or let it find the server on your network.

The full guide, with troubleshooting, updating and uninstalling, is in [docs/install.md](docs/install.md).

## Keep it private

Switchboard has no passwords. Anyone who can reach its ports (8130 to 8132) can see and control your chats. Keep
it on your home network or your own [Tailscale](https://tailscale.com) network, and never forward those ports to
the internet.

## What's in this repository

| Folder | What |
|---|---|
| `server/` | The server (Python) |
| `agent/sb.py` | The helper that runs on each computer: Claude Code hooks, status line, and the commands the server sends |
| `native/` | The desktop app, the terminal hub and `switchboard-node` (Rust) |
| `scripts/install/` | Installers and first-run setup |
| `tests/` | Tests |
| `docs/` | Install guide, how the pieces fit, and the decision log |
| `branding.toml` | The app's name, ids and colors, in one place |

## Building and contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) to build it, run the tests and send a change. Report security problems as
described in [SECURITY.md](SECURITY.md).

Switchboard is not made by or affiliated with Anthropic. Claude and Claude Code are trademarks of Anthropic.

## License

[MIT](LICENSE)
