---
description: What each section of Switchboard's Settings holds, from your computers and their Claude folders to notifications, Chrome, the budget, updates and the opt-in AI features.
---

# Settings

++cmd+comma++ opens **Settings**. Most of it lives on the server, so it's the same on every computer you use the
app on.

## Appearance

Light, dark, or the system's. Synced, so your other computers match.

## Terminal

The color scheme, the default shell for new terminals, the text size, and the cursor's shape and blink.
[Terminals](terminals.md#look)

## New sessions

- **Summaries:** who writes them for Start fresh and Continue: **Opus** (the most careful) or **Sonnet** (quicker,
  and less of your plan).
- **Saved setups:** the combinations you saved in New chat, which it offers along with Defaults and Last used.

## Machines

Every computer Switchboard watches, in board order: rename, reorder, remove, and **Add a machine**. Open one for:

- **Claude folder:** where new chats start.
- **Beyond the Claude folder:** whether chats and Files may go anywhere in its home folder, or stay in the Claude
  folder and the places added there.
- **Capabilities:** tools like Xcode, an Android SDK or a GPU, which [Auto](chats.md#auto) goes by, and a box
  for anything else it can do that Switchboard can't find by itself.
- **MCP servers:** read from Claude Code's settings there (names and commands only; secrets never leave the
  computer), with **Add to library** to share one with every chat.

**Server addresses** lists every address the app knows for the server, marks the ones it learned, and lets you
remove any of them.

## Library

Tools you built, kept on the server so a chat on any computer can use them. A tool is a Claude Code plugin: a
skill or an MCP server.

- **Import...** a skill or an MCP server one of your computers already has.
- **New tool...** describes what it should do and picks a computer to build it on; a chat there builds it, tests
  it and publishes it to the library. You can follow it on the board.
- Each tool has **On for every new chat**, **Include in Auto**, **When to use**, the computers it's on, and every
  version, with **Make latest** to go back.
- **Keys it needs** are names only: each computer keeps its own values in its system keychain
  (`sb.py secret set <tool> <NAME>`).

A chat gets a tool when you attach it (the chat restarts, and reads its context again) or when the tool is on for
every new chat. Chats can also publish and list tools themselves, through the `switchboard` MCP server the helper
adds to Claude Code.

## Notifications

- **On this Mac** (or **On this computer**): **When a chat needs you**, a plain notification with the chat's name and computer. Click
  it to open the board. It doesn't show while a Switchboard window is in front.
- **On your iPhone:** a switch for each kind: **Held messages**, **Finished tasks**, **Chats not in use** (at
  most once a day) and **Hard limit**. [iPhone notifications](apps.md#iphone-notifications)

Toasts that leave the corner wait in **Notices**, at the right of the status bar, until you ignore them.

## Chrome machine

For chats that use [Claude in Chrome](https://claude.ai/chrome). It lists which computers have Chrome, the
extension, and a live connection. Each chat picks its own Chrome computer, in New chat's **Claude in Chrome**
line or in the chat's details, and Start fresh and Continue keep it. **Auto** sends a Chrome chat to its Chrome
computer.

## Budget

A daily share of your weekly limit, a 5-hour cap, and the hard limit. [Usage and budget](usage.md)

## Updates

Whether this build gets updates, what it last found, and **Check now**. [Updates](updates.md)

## Server

The computer that runs Switchboard: its version, whether each part is running, free disk space, **Update
server** when a newer one is out, **Restart**, its logs, and **Copy details**. [Updates](updates.md#the-server)

## AI and your Claude setup

Everything here is off on a new server until you turn it on. A server set up before these switches existed
had them all turned on once, so look them over.

| | |
|---|---|
| **Summarize waiting chats** | Claude sums up what a waiting chat is asking. Its last message goes to Claude |
| **Name new chats** | Claude names a new chat from its first message |
| **Let Claude pick for Auto** | Claude picks the computer for Auto from the task and each computer's tools, free memory and repo names |
| **Subagent model rule** | A short rule in `~/.claude/CLAUDE.md` on each computer: Haiku for lookups, Sonnet for search and reading |
| **Explore agent on Sonnet** | `~/.claude/agents/Explore.md` on each computer |
| **Compact at 300k tokens** | `CLAUDE_CODE_AUTO_COMPACT_WINDOW=300000` in each computer's Claude Code settings |

The first three use your own Claude plan. Turning a setup change off removes only what Switchboard added.
[Privacy and security](privacy.md#what-leaves-your-network-and-when)

## About

In the app menu: the version and build, the server it talks to, **Copy version info**, **What's new**, **Show
log**, **Copy diagnostics** and the open source licenses.
