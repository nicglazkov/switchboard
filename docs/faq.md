---
description: Common questions about Switchboard, what it is, what it costs, what it needs, and what it does with your chats.
---

# FAQ

??? question "Is Switchboard made by Anthropic?"
    No. It's an independent open source project that works with [Claude Code](https://claude.com/claude-code).
    It runs Claude Code on your computers and reads what Claude Code writes there.

??? question "What does it cost?"
    Nothing. It's free and open source, under the GNU AGPL 3.0. Your chats use your own Claude plan, as they would without Switchboard, and the
    board helps you use less of it.

??? question "Do I need more than one computer?"
    No. One computer can run the server, the app and Claude Code. Switchboard still sorts your chats, tells you
    what going back to each one costs, and holds messages that would read a big cold chat again.

??? question "Which computer should run the server?"
    One that stays on and that your other computers can reach: a Mac mini, a desktop that never sleeps, a small
    Linux box. A Mac or Linux runs all of it. A Windows server works, but without terminals or linking computers
    with a code. [Install](install.md)

??? question "Does it work away from home?"
    Yes, wherever the app can reach the server: a VPN like Tailscale, or the same network. The app remembers every
    address the server knows and tries the last one that worked first. Computers linked with a code reach the
    server the same way, from behind any router.

??? question "Does it change how Claude Code works?"
    The helper adds hooks and a status line to Claude Code's settings on each computer (and saves the status line
    you had), plus a `switchboard` MCP server. That's how it knows each chat's state and cache, and how it holds a
    message. `sb.py uninstall` takes out only what it added. The setup changes in **Settings, AI and your Claude
    setup** stay off until you turn them on.

??? question "Does it send my chats anywhere?"
    No. Everything stays on your computers and your server. The exceptions: summaries you ask for (Start fresh,
    Continue) and the opt-in AI features send chat text to Claude on your own plan, like any chat; the server
    looks at GitHub for updates when an app checks; and iPhone notifications go through Apple if you set them
    up. [Privacy and security](privacy.md)

??? question "Is there a password?"
    No. Anyone who can reach the server's ports can see and run your chats, so keep it on a network only you can
    reach. [Why](privacy.md#no-passwords-on-purpose)

??? question "Will it stop my chats?"
    Only when you say so, or when you turn on the budget's hard limit. It suggests stopping chats that have sat
    idle for days, and never suggests one you marked **Keep running** or one open in a terminal.
    [Tidy up](board.md#tidy-up)

??? question "Where are the numbers for my plan from?"
    From Claude Code itself: its status line reports how much of the 5-hour window and the week you've used.
    Switchboard doesn't call any Anthropic service for them. [Usage](usage.md)

??? question "Is there an iPhone or Android app?"
    There is an iPhone app, but it isn't on the App Store yet. There is no Android app.
    [The apps](apps.md#iphone)
