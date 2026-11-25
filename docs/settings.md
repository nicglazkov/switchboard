---
description: Every section and setting in Switchboard's Settings window, with what each one does, its default, and whether it applies to every computer or only the one you're at.
---

# Settings

<p class="sb-meta">Last validated: Nov 24, 2026</p>

Settings is where you change how Switchboard looks and behaves. Press ++cmd+comma++ (++ctrl+comma++ on Windows
and Linux) to open it.

Most settings are kept on the home computer, which is the one of your computers that runs Switchboard, so they
are the same on every computer you use the app on. A setting that is kept on one computer says **This computer
only** in the app, and in the tables on this page.

Settings uses the app's own words: a computer is a **machine** there, and the home computer is the **server**.

## Sections

| Group | Section | What it holds |
|---|---|---|
| **General** | [**Appearance**](#appearance) | Light or dark |
| | [**Notifications**](#notifications) | What reaches you, on this computer and on an iPhone |
| **Chats** | [**Chats**](#chats) | Summaries, saved setups, stopping idle chats |
| | [**Context window**](#context-window) | Where long chats are compacted |
| | [**Terminal**](#terminal) | Colors, shell, text size, cursor |
| | [**Budget**](#budget) | The daily budget and the hard limit |
| **Claude** | [**Library**](#library) | Your shared skills and MCP servers |
| | [**Chrome**](#chrome-machine) | Which computers have Claude in Chrome |
| | [**AI and your Claude setup**](#ai-and-your-claude-setup) | The opt-in features |
| **Home computer** | [**Devices**](#devices) | Paired computers and phones. Since version 0.9.0, with device keys |
| | [**Server**](#server) | The home computer |
| **Machines** | [**Machines**](#machines) | Your computers and their folders, with a line for each computer under it |
| Bottom of the sidebar | [**Updates**](#updates) | New versions of the app |
| | [**About**](#about) | The version, the log and the licenses |

## Appearance

| Setting | Choices | Default | Applies to |
|---|---|---|---|
| **Theme** | **Light**, **System**, **Dark** | **System** | Every computer, and the iPhone app |

++cmd+shift+l++ switches between light and dark from anywhere.

## Notifications

| Setting | Group | Default | Applies to |
|---|---|---|---|
| **When a chat needs you** | **On this Mac**, or **On this computer** | On | This computer only |
| **Held messages** | **On your iPhone** | On | Every iPhone |
| **Finished tasks** | **On your iPhone** | On | Every iPhone |
| **Chats not in use** | **On your iPhone** | On | Every iPhone |
| **Hard limit** | **On your iPhone** | On | Every iPhone |

The iPhone switches do nothing until the home computer has a push key. Until then the page says **iPhone
notifications aren't set up**. For more information, refer to [Notifications](features/notifications.md).

## Chats

| Setting | Choices | Default | What it does |
|---|---|---|---|
| **Who writes them**, under **Summaries** | **Opus**, **Sonnet** | **Opus** | Picks the model that writes the summary when a chat starts fresh or a thread is compacted. Opus is the most careful. Sonnet is quicker and uses less of your plan |
| **Saved setups** | **Remove** for each | None saved | Lists the setups you saved when starting a chat: a model, effort, mode and flags. **Remove** asks you to click again |
| **Stop idle chats** | **Never**, **2 hours**, **6 hours**, **1 day** | **6 hours** | Stops a chat that finished its reply and has been left alone this long, to free its memory. **Resume** brings it back |

**Stop idle chats** applies to every computer. It never stops a chat that is working, waiting on you, running a
command or a scheduled task, starred, set to **Keep running**, or open in a tab. For more information, refer to
[Tidy up](features/tidy-up.md).

## Context window

**Compact long chats at** is one slider. It sets how much of a long chat Claude Code keeps word for word before
it compacts the chat, which means replacing the older part with a summary. The same value goes to every computer
and covers every model.

| Stop | Name | What it means |
|---|---|---|
| **150k** | Lean | Chats stay small, so coming back to a cold one costs the least. Compacts soonest |
| **200k** | Standard | The size of a model's window before the 1M models |
| **300k** | Balanced | The recommended value, and the default once the setting is on |
| **500k** | Long memory | Half a million tokens stay word for word, and a cold chat that size re-reads all of it |
| **Default** | Claude Code's default | Switchboard sets nothing, and Claude Code compacts where it does by itself |

The setting is off on a newly installed home computer, like the other changes to your Claude setup. A model with
a smaller window than the value compacts at its own window, so one value is safe for every model. To give each
model its own window, pick **Default** and use `/autocompact` in Claude Code.

**Follows from this** shows two things that move with the slider and aren't settings of their own:

- **Context meters fill at** the value, or the model's window when that is smaller.
- **Start fresh is suggested from** nine tenths of the value: 270k at 300k.

**On each computer** lists what every computer has. Switchboard writes `CLAUDE_CODE_AUTO_COMPACT_WINDOW` to a
computer's Claude Code settings only where no compact window was chosen by hand, and only ever changes or
removes what it wrote.

| Line | What it means |
|---|---|
| **Set on this computer: 500k, left alone.** | You set a window there yourself. The slider never touches it |
| **Older helper** | That computer's Switchboard helper is from before the slider. Update it there |
| **Offline. Changing to 300k when it is back.** | The computer follows when it's online again |

A chat picks the value up the next time it starts.

## Terminal

| Setting | Choices | Default | Applies to |
|---|---|---|---|
| Color scheme | Twelve schemes, or **Match the app** | Converge Dark | Every computer |
| **Default terminal** | Any shell found on this computer | The first in the list | This computer only |
| **Text size** | 9 to 24 | 13 | This computer only |
| **Cursor** | **Block**, **Bar**, **Underline** | **Block** | Every computer |
| **Blink** | On or off | On | Every computer |
| **Stop a chat when its tab closes** | On or off | Off | This computer only |

For more information, refer to [Terminals](terminals.md#look).

## Budget

| Setting | Range | Default | What it does |
|---|---|---|---|
| **Daily budget** | A percent of your weekly limit | 14.3%, which spreads the week evenly over 7 days | Switchboard tells you when a day goes over it, and a chat tells you once a day |
| **5-hour cap** | A percent of the 5-hour window | Off | The same, for the 5-hour window |
| **Hard limit** | On or off | Off | Blocks new messages and stops running background chats when a limit is reached. You can resume them |
| **Daily limit**, under **Hard limit** | A percent of the week for each day | 20% | The share of the week that triggers the hard limit |
| **5-hour limit**, under **Hard limit** | A percent of the 5-hour window | Off | The share of the 5-hour window that triggers the hard limit |

The minus and plus buttons move to whole numbers. When the hard limit has been reached, the page says so and
offers **Lift for today**, which holds until midnight. **Undo** puts the limit back.

For more information, refer to [Daily budget](features/daily-budget.md) and [Usage and budget](usage.md).

## Library

The library holds the tools you built, kept on the home computer so a chat on any computer can use them.

| Control | What it does |
|---|---|
| **Import...** | Adds a skill or an MCP server that one of your computers already has |
| **New tool...** | Starts a chat that builds a tool and publishes it |
| **On for every new chat** | Off by default. New chats on every computer start with the tool |
| **Include in Auto** | On by default. Chats that start with Auto tools get the tool |
| **When to use** | One line that tells Claude when to reach for the tool |
| **Keys it needs** | The names of the tool's keys. **Set key...** saves a value on one computer |
| **Machines** | Which computers have the tool, and which version |
| **Versions** | Every version, with **Make latest** to go back |
| **Delete** | Removes the tool from every computer and every chat. Asks you to click again |

For more information, refer to [Library](features/library.md).

## Chrome {#chrome-machine}

This section has no settings. It lists which of your computers have Chrome, the Claude in Chrome extension and a
live connection to it, for chats that use [Claude in Chrome](https://claude.ai/chrome).

There is no Chrome computer for all chats. Each chat picks its own, in the **Claude in Chrome** line when you
start it or in the chat's details, and **Start fresh** and **Continue** keep that choice.

## AI and your Claude setup

Everything here is off on a newly installed home computer until you turn it on.

| Setting | Group | What it does when on |
|---|---|---|
| **Summarize waiting chats** | **Sends chat text to Claude** | Uses your Claude plan to sum up what a waiting chat is asking. Its last message goes to Claude. When off, the card shows the chat's own words |
| **Name new chats** | **Sends chat text to Claude** | Uses your Claude plan to name a new chat from its first message. When off, a new chat is named from its first few words |
| **Let Claude pick for Auto** | **Sends chat text to Claude** | Uses your Claude plan to pick a computer for tasks sent to Auto. The task and each computer's tools, free memory and repository names go to Claude. When off, Auto goes by rules |
| **Subagent model rule** | **Changes to each machine's Claude setup** | Adds a short rule to `~/.claude/CLAUDE.md` on each computer: Haiku for single lookups, Sonnet for search and reading, the default model for design and code |
| **Explore agent on Sonnet** | **Changes to each machine's Claude setup** | Writes `~/.claude/agents/Explore.md` on each computer, so the Explore agent runs on Sonnet and only reads. Your own `Explore.md` is left alone |

Turning a setup change off removes only what Switchboard added. Where long chats are compacted is also a change
to each computer's Claude Code settings. It has its own section, [Context window](#context-window).

For more information, refer to [Privacy and security](privacy.md).

## Machines

The section lists every computer Switchboard watches, in board order. You can rename, reorder and remove
computers here, and **Add a machine**. Refer to [Add your computers](machines.md).

Open a computer for its own page.

| Group | Setting | Default | What it does |
|---|---|---|---|
| **About this machine** | **Rename** | The name it was added with | Changes the name shown on the board. Its **Id** never changes |
| **Claude folder** | **Choose**, **Type a path** | Not picked, so chats start in the home folder | Sets where new chats start on that computer |
| **Beyond the Claude folder** | **Anywhere in home** | On | Lets chats and Files use any folder in that computer's home folder. Claude still asks before it works in a folder for the first time |
| **Capabilities** | **Let Auto send work here** | | Lets Auto count this computer in, with the tools Switchboard found there |
| **Capabilities** | **This machine can also do** | Empty | Anything detection missed, in your own words. Auto reads these too |
| **MCP servers** | **Add to library** | | Shares one of that computer's MCP servers with every computer. Only names and commands show, and secrets never leave the computer |
| | **Remove this machine** | | Switchboard stops watching it. Its chats and files stay on it. Asks you to click again |

For more information about the first two groups, refer to
[Places and folders](concepts/places-and-folders.md).

## Devices

!!! note ""
    Devices arrived with device keys, in Switchboard 0.9.0. In the current app, **Server** sits in the
    **Machines** group and there is no **Devices** section.

The section shows on the home computer, and on any computer that is paired with one. It is where you turn keys
on, pair a computer or a phone, and remove one.

| Control | What it does |
|---|---|
| **Add a device** | Makes a code, a link and a QR code for one device. They work once, for ten minutes |
| **Copy code**, **Copy link** | Copies what you carry to the other device |
| **Pair this computer** | On a computer that connects by address, gets a code from the home computer and uses it, with nothing to type |
| **Paired devices** | Every device the home computer accepts, with whether it is connected. **Remove** takes one off the list |
| **While the home computer is away** | On a paired computer: its copy of the list, with **Block** and **Unblock** for each device |

For more information, refer to [Pair a computer](features/pair-a-computer.md#pair-from-the-desktop-app) and
[When the home computer is away](features/when-home-is-away.md).

## Server

The section shows the home computer: its version, whether each of its parts is running, free disk space and its
logs.

| Control | What it does |
|---|---|
| **Check now** | Looks for a newer version of the home computer's software |
| **Update server** | Installs it, when one is ready |
| **Restart the server** | Restarts Switchboard on the home computer. Asks you to click again |
| **Show logs** | Shows what the home computer has been doing |
| **Copy details** | Copies the version, the system and how it runs, for a bug report |
| **Server addresses** | This computer only. The addresses this app tries when it looks for the home computer. You can add one and remove any |

For more information, refer to [Updates](updates.md) and [Install Switchboard](install.md).

## Updates

| Control | What it does |
|---|---|
| **Check now** | Looks for a new version of the app now. The app also looks by itself once a day |
| **Try it again** | Offers again a version that was put back because it didn't start |

Updates never change your settings or what you chose when you set Switchboard up. A build that can't update
itself says so here. For more information, refer to [Updates](updates.md).

## About

**About** opens a window of its own with the version and build, **Copy version info**, **What's new**, **Show
log**, **Copy diagnostics** and **Open source licenses**.
