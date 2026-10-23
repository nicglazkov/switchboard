<div align="center">

<img src="docs/assets/icon.png" alt="" width="96">

# Switchboard

**One board for the Claude Code chats on all your computers.**

See which chats need you, answer them, open their terminals and start new ones, on whichever computer you pick.
It runs on your own machines, with no account and no cloud service.

[![CI](https://github.com/nicglazkov/switchboard/actions/workflows/ci.yml/badge.svg)](https://github.com/nicglazkov/switchboard/actions/workflows/ci.yml)
[![Docs](https://github.com/nicglazkov/switchboard/actions/workflows/docs.yml/badge.svg)](https://nicglazkov.github.io/switchboard/)
[![Release](https://img.shields.io/github/v/release/nicglazkov/switchboard?label=release&color=0C7480)](https://github.com/nicglazkov/switchboard/releases/latest)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-0C7480)](LICENSE)

**[Documentation](https://nicglazkov.github.io/switchboard/)** ·
[Download](https://github.com/nicglazkov/switchboard/releases/latest) ·
[Install](#install) ·
[Changelog](CHANGELOG.md)

<br>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/board-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshots/board-light.png">
  <img alt="The Switchboard board: chats from three computers sorted into Needs you, Working, Warm now, Gone cold and Quiet" src="docs/screenshots/board-light.png" width="900">
</picture>

</div>

## What it does

You run Claude Code on a desktop, a laptop and a box in the closet. Each chat lives in its own terminal on its own
computer, and the one waiting for you is never the one in front of you. Switchboard puts them all on one board.

- **Every chat, sorted by what it needs.** Each computer gets a row; each chat sits in **Needs you**, **Working**,
  **Warm now**, **Gone cold** or **Quiet**. Answer a question right from its card.
- **Know the cost before you go back.** A cold chat reads its whole history again on the next message. Switchboard
  compares resuming it with starting fresh from a summary, suggests the cheaper one, and holds a message that would
  re-read a big cold chat until you confirm.
- **Terminals from anywhere.** Open any chat's terminal in a tab, a window or a floating sheet, even when its
  computer is behind another router. Your own shells, tab groups and drag-and-drop files too.
- **Start a chat on any computer.** Pick the computer (or let Auto pick), the folder, the model and the effort.
- **History, files and usage.** Search every chat on every computer, browse their folders, and watch your plan's
  5-hour and weekly usage against a daily budget.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/details-dark.png">
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshots/details-light.png">
  <img alt="A cold chat's details: start fresh or resume, with what each one costs" src="docs/screenshots/details-light.png" width="800">
</picture>

## Install

Switchboard has three parts: a **server** on one computer that stays on, a small **helper** on each computer where
you use Claude Code, and the **app** wherever you want to see the board. One computer can be all three.

**1. The server**, on a Mac or Linux computer that stays on:

```sh
curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash
```

Or let the app do it: when it finds no server, the Mac and Linux apps offer **Run the server on this computer**.

**2. Your computers.** Setup shows a one-time code and a line to paste on each Mac, Linux or Windows computer
where you use Claude Code.

**3. The app.**

| | |
|---|---|
| **Mac** | [`Switchboard-<version>.dmg`](https://github.com/nicglazkov/switchboard/releases/latest), macOS 11 or later. Signed and notarized |
| **Windows** | [`Switchboard-Setup-<version>.exe`](https://github.com/nicglazkov/switchboard/releases/latest), installs without administrator rights |
| **Linux** | [`Switchboard-linux-x86_64.tar.gz`](https://github.com/nicglazkov/switchboard/releases/latest) |

On your home network the app finds the server by itself. Updates come through your server from this repository's
releases, signed, and install only when you say so.

You need [Claude Code](https://claude.com/claude-code) on each computer that runs chats, and Python 3.9 or newer
on the server and on those computers. **[The full install guide](https://nicglazkov.github.io/switchboard/install/)**
covers a Windows server, other options and uninstalling.

## Documentation

| | |
|---|---|
| [Get started](https://nicglazkov.github.io/switchboard/install/) | Install, add computers, the apps |
| [Use it](https://nicglazkov.github.io/switchboard/board/) | The board, chats and costs, terminals, history and files, usage, shortcuts, settings |
| [Help](https://nicglazkov.github.io/switchboard/troubleshooting/) | Troubleshooting, FAQ, updates, privacy, getting help |
| [Reference](https://nicglazkov.github.io/switchboard/commands/) | Commands, files and ports, changelog |
| [Develop](https://nicglazkov.github.io/switchboard/architecture/) | How it works, building from source, contributing |

## Privacy

Your chats, files and history stay on your computers and your server. Switchboard sends no usage data anywhere.
What does leave your network: the server's checks for updates on GitHub, iPhone notifications through Apple
if you set them up, and the AI features you turn on, which use your own Claude plan.

There is no password: anyone who can reach the server's ports can see and run your chats. Keep it on a network only
you can reach, like your home network or a VPN such as Tailscale.
[Privacy and security](https://nicglazkov.github.io/switchboard/privacy/) has the details, and
[SECURITY.md](SECURITY.md) how to report a problem.

## Contributing

Bug reports and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) and
[building from source](https://nicglazkov.github.io/switchboard/building/).

## License

[GNU AGPL-3.0](LICENSE). Switchboard is not made by or affiliated with Anthropic. Claude and Claude Code are
trademarks of Anthropic.
