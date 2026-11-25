---
description: The library keeps the skills and MCP servers you built in one place, so a chat on any of your computers can use them, with each computer holding its own keys.
---

# Library

<p class="sb-meta">Last validated: Nov 22, 2026</p>

The library lets a chat on any of your computers use the tools you built on one of them. A tool is a Claude Code
plugin: a skill, an MCP (Model Context Protocol) server that connects Claude to something else, or both in one
folder.

## Why it matters

Claude Code keeps skills and MCP servers in the settings of one computer. A tool you set up on the desktop isn't
there when a chat runs on the laptop, and copying it by hand leaves you with versions that drift apart.

## Benefits

- **One copy to maintain**: the library holds every version of every tool, and each computer gets the one marked
  latest.
- **Keys stay where they're used**: a tool's keys are kept in each computer's own keychain. Only the names of
  the keys travel.
- **Nothing is written into your Claude setup**: a chat gets a tool only because Switchboard started it with that
  tool. Your own Claude Code settings and skills folders aren't changed.
- **Chats can add tools themselves**: a chat can build a tool, test it and publish it to the library.

## Use cases

- **A skill you rely on**: import a skill from the computer where you wrote it and turn it on for every new chat.
- **A connector with a key**: share an MCP server with every computer, and set its key once on each.
- **A tool you don't have yet**: describe it, and a chat builds it on the computer you pick.

## How it works

The library is kept on the home computer, which is the one of your computers that runs Switchboard. Each of your
computers holds copies of the tools it uses.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/library-three-computers.svg"
<figcaption>Figure 1. Every computer takes its tools from the one library, but each has only the tools it uses, and
each needs its own keys.</figcaption>
</figure>

In the figure, the library holds three tools. The desk Mac has all three. The laptop has `pdf-tools`, and it has
`web-fetch` but not the key that tool needs. The closet PC has only `pdf-tools`.

A computer gets a tool in one of two ways: a chat that uses the tool starts there, or you turn the tool on for
that computer. When you publish a new version, every computer that has the tool gets it. A computer that is
offline catches up when it's back, and a chat that is already running keeps the version it started with.

A chat gets a tool in one of three ways:

- **It's on for every new chat**: every new chat on every computer starts with it.
- **The chat started with Auto tools**: it gets every tool that has **Include in Auto** on and has its keys on
  that computer.
- **You attached it**: you picked the tool when you started the chat, or attached it later in the chat's
  details. Attaching to a running chat restarts it, and the chat reads its context again.

The figure doesn't draw the keys themselves. A key's value is saved on the computer that uses it: in the macOS
Keychain, in Windows Credential Manager, or on Linux in the system keyring, or in a file only you can read
when there is none. The value never goes to the home computer.

## Use the library

1. Add a tool.
2. Decide which chats get it.
3. Set its keys on each computer.

### Prerequisites

Before you can add a tool, you must:

- Have at least one computer online. This is because a tool is packed, or built, on one of your computers.

### Add a tool

Open **Settings** and select **Library**. There are three ways in.

- **Import...**: pick a computer, then a skill or an MCP server it already has, and select **Add**. A skill
  brings its whole folder. An MCP server keeps its command, but the values in its settings stay behind, and each
  one becomes a key to set on every computer.
- **New tool...**: type a **Name** and **What it should do**, pick a computer under **Build it on**, and select
  **Start building**. A chat on that computer builds the tool, tests it and publishes it. You can follow that
  chat on the board.
- **Add to library**: on a computer's page under **Machines**, next to one of its **MCP servers**.

A tool's name is 1 to 40 lowercase letters, digits and dashes. Importing or publishing under a name that already
exists adds a new version of that tool.

### Decide which chats get it

Each tool in the library has its own card.

| Setting | What it does | Default |
|---|---|---|
| **On for every new chat** | New chats on every computer start with the tool | Off |
| **Include in Auto** | Chats that start with Auto tools get the tool | On |
| **When to use** | One line that tells Claude when to reach for the tool. Without it, Claude goes by the description | Not set |
| **Machines** | Puts the tool on a computer, or takes it off | Off for each computer |
| **Versions** | Lists every version. **Make latest** goes back to an older one | The newest is **Latest** |

To give a tool to one chat only, pick it under **Tools** when you start the chat, or select **Attach tool...**
in a chat's details. A chat can have up to 32 tools.

### Set its keys on each computer

**Keys it needs** on a tool's card lists the names of its keys. Under **Machines**, a computer that lacks one
says **needs** followed by the key's name.

1. Select **Set key...** beside the computer.
2. If the computer is the Mac you're at, paste the value and select **Save**. It goes straight into that Mac's
   keychain.
3. For any other computer, select **Copy** to copy the command that Switchboard shows, and run it in a terminal
   on that computer. It asks for the value without showing it.

    ```
    python3 ~/.claude/switchboard/sb.py secret set example-tool EXAMPLE_KEY
    ```

    Replace `example-tool` with the tool's name and `EXAMPLE_KEY` with the key's name. Switchboard fills in both
    in the command it shows.

## Turn off or delete a tool

To take a tool away from one computer, turn it off for that computer under **Machines** on the tool's card. To
stop giving it to new chats, turn off **On for every new chat** and **Include in Auto**.

**Delete** takes the tool off every computer and out of every chat, and removes all its versions. It asks you to
click again first.

## Limitations

- A tool can be up to 25 MB packed.
- A chat can have up to 32 tools.
- A tool that reaches a running chat needs the chat to restart before the chat can use it.
- Keys can be saved from the app only into the keychain of the Mac you're at. Every other computer needs the
  command.
- A skill runs its scripts through Claude's own shell, so Claude can read a key that a skill uses. The library
  says so on such a tool. Make it an MCP server to keep the key hidden.

## Troubleshooting

### A computer says it needs a key

The tool is on that computer, but its key isn't. Set the key as described in
[Set its keys on each computer](#set-its-keys-on-each-computer). Until then:

- a chat that started with Auto tools leaves the tool out, and is told which key is missing;
- a chat you attached the tool to has it, but the tool's MCP server doesn't start.

### A tool says a version is on its way

The computer has an older version, or none yet. It gets the latest one the next time it's online.

### A new tool doesn't show in a running chat

A chat loads its tools when it starts. Attach the tool in the chat's details and let the chat restart.
