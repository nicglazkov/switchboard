---
description: Open any Claude Code chat's terminal from the Switchboard app, in tabs, windows or a floating sheet, plus your own shells, tab groups and drag and drop.
---

# Terminals

Every chat's terminal opens right in the app, on whatever computer it runs: in a tab, in its own window, or as a
floating sheet over the board. Your own shells open the same way.

## Open a chat's terminal

| How | Where it opens |
|---|---|
| ++enter++ on a card, or a double-click | A tab |
| ++shift+enter++ | Its own window |
| ++alt+enter++ (++option+enter++ on a Mac) | A floating sheet over the board |
| ++cmd+o++, then type a few letters | A tab, for any chat |

A stopped chat is resumed into the tab. The terminal is the chat itself (Claude Code's `claude attach`), so
closing a window only detaches it, while closing a **tab** stops the chat unless you marked it **Keep running**.
Typing `exit` (or `/exit`) closes its tab.

!!! info "Behind another router"
    A computer linked with a code keeps one outgoing link to your server's hub, and its terminals travel over
    that link. No ssh and no port forwarding is needed, wherever the computer is.

## Your own shells

++cmd+t++ opens a terminal with your default shell, on the computer you're at. Nothing goes through the server.
The **+** at the end of the tab row lists every shell it found:

- **Mac and Linux:** your login shell first, then zsh, bash, fish, nu, pwsh and the others in `/etc/shells`.
- **Windows:** PowerShell 7, Windows PowerShell, Command Prompt, Git Bash, and each WSL distribution.

Pick the default in **Settings, Terminal**. On a Mac and Linux, closing a shell that's running a program asks
first.

## Tabs

Right-click a tab for **Star tab**, **Rename chat**, the group options, **Move to a new window**, **Show as a
floating sheet** and **Close tab**. Drag a tab to reorder it, or down out of the row to give it its own window.
++cmd+1++ is the board, ++cmd+2++ to ++cmd+8++ the tabs in order, ++cmd+9++ the last one.

With many tabs, each one shrinks to a dot before the row scrolls, and the one you're on keeps its title.

### Groups

**Add to new group** in a tab's menu starts a group in the next free color: Blue, Red, Yellow, Green, Pink,
Purple, Cyan or Grey. Type its name on the chip. Click the chip to fold the group; right-click it to rename, recolor,
move, ungroup or close it. Groups follow a chat through Start fresh, Continue and Resume, and are the same on
every computer you use the app on (which tabs are open, and folding, stay on each one).

### Rename a chat

Double-click a tab's title, type, and press ++enter++ (++esc++ puts it back). The name shows everywhere, and in
Claude Code's own `/resume` list. [Names](chats.md#names)

## Drag files in

Drop files on a chat's terminal to send them to its computer: each one is uploaded there (up to 20 MB) and its
path is typed into the chat. On your own shell, a dropped file types its path at the cursor. You can also paste
files, or use **Attach**.

## When the connection drops

A dropped terminal reconnects by itself, trying for about five minutes, and keeps what you could see and scroll
back to. **Reconnect now** tries right away. If the screen ever looks wrong, **Redraw** (++cmd+shift+r++) asks for
a fresh copy of it.

## Look

**Settings, Terminal** has the color scheme (hover one to preview it in every open terminal), the text size (9 to
24, or ++cmd+plus++ and ++cmd+minus++ in a terminal), the cursor shape (block, bar or underline) and blinking.
Schemes: Converge Dark and Light, Tokyo Night and Tokyo Night Day, Nord, Dracula, Gruvbox Dark and Light,
Solarized Dark and Light, GitHub Light, Catppuccin Latte, or **Match the app**. These follow you to your other
computers.
