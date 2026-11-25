---
description: Start chats on any computer, and know what going back to a chat will cost before you do. Warm and cold, Resume, Start fresh, the guard and Continue.
---

# Chats and what they cost

<p class="sb-meta">Last validated: Nov 22, 2026</p>

A chat is one Claude Code conversation running on one of your computers. Switchboard lets you start chats on
any computer, tells you what your next message in each would cost, and offers a cheaper way back when a long
chat has sat unused.

## Warm and cold

Claude keeps a ready copy of a recent chat, known as the cache, for 5 minutes or 1 hour after each reply.

- **Warm**: the cache is still there. Your next message reads the chat from it, at about a tenth of the price.
- **Cold**: the cache has run out. Your next message makes Claude read most of the chat again in full.

On the board, a warm chat sits in <span class="sb-pill warm">Warm now</span> and counts down. A cold chat of
150k tokens or more moves to <span class="sb-pill cold">Gone cold</span>.

How much going cold matters depends on the size of the chat:

| Size of the chat | Going back to it cold |
|---|---|
| Under 80k tokens | Cheap anyway. Resume it. |
| 80k to 150k | Reads it all again. Start fresh is cheaper if you don't need the whole history. |
| 150k or more | Reads it all again at full price. The guard holds your message first. |

For more information, refer to [Warm and cold](concepts/warm-and-cold.md) and
[What a chat costs](concepts/what-a-chat-costs.md).

## Resume or start fresh

A quiet chat has two ways back. **Resume** reopens it as it is. **Start fresh** writes a summary of 500 to 900
words and carries on in a new chat from that summary. A chat's details show what each would cost and mark one
as **Suggested**.

Start fresh pays off from about 80k tokens. It makes a single cold message much cheaper. Measured over a month
of real use, it saved about 1.2% of all usage, or about 4% counting later replies.

For more information, refer to [Resume or start fresh](features/resume-or-start-fresh.md).

## The guard for big cold chats

A message you type into a cold chat of 150k tokens or more is held before it is sent. You see what it would
cost and choose **Send anyway** or **Start fresh**. Prompts that nobody typed, such as scheduled ones, are
never held.

For more information, refer to [The cold chat guard](features/cold-chat-guard.md).

## Start a new chat

Press ++n++ on the board to open **New chat**. Pick a computer, or **Auto** to let Switchboard pick, then the
folder and what the chat should work on.

For more information, refer to [Start a chat](features/start-a-chat.md).

### Auto

**Auto** picks the computer for you: one with the tools your message asks for, then one with a connected phone,
then one that has the repository you name, then the one with the most free memory.

For more information, refer to [Start a chat](features/start-a-chat.md#how-it-works).

## Continue on another computer

**Continue on another machine** writes a summary of a chat where it runs and starts a new chat from it on a
different computer, with the same model, effort and permissions.

For more information, refer to [Continue on another computer](features/continue-on-another-computer.md).

## Change a chat's flags

**Flags**, on a card, in its details and on its terminal's bar, shows what a chat runs with and lets you change
it.

- **Changes at once**: the model, the effort, turning plan mode on, and adding extra folders are typed into
  the chat. Nothing restarts.
- **Needs a relaunch**: permissions, Claude in Chrome, computer use and other flags are read when Claude Code
  starts. The chat relaunches and carries on with its history.
- **Can't change**: a worktree is set when a chat starts.

## Names

A new chat is named from the first few words of its first message. With **Name new chats** turned on in
Settings, Claude names it from that message. **Rename chat** changes the name everywhere at once, including in
Claude Code's own `/resume` list.

For more information, refer to [Terminals](terminals.md#rename-a-chat).
