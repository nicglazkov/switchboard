---
description: Switchboard stops idle Claude Code chats to free memory, asks about the ones it isn't sure of, and never stops a chat that is working, waiting on you, starred or kept running.
---

# Tidy up

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Tidy up lets you free the memory that idle chats hold. Switchboard stops the chats that cost nothing to stop,
asks you about the rest, and leaves alone any chat that you starred, marked **Keep running**, or that still
has something to do.

## Why it matters

A chat that sits open and does nothing still holds memory on its computer. A few of them are harmless. A week
of them can slow a computer down, and it is hard to tell from a list of processes which ones are safe to end.

## Benefits

- **Memory comes back by itself**: a chat that finished its reply and was left alone is stopped after 6 hours.
- **Nothing is lost**: a stopped chat keeps its whole history, and **Resume** brings it back where it was.
- **No extra cost**: a chat is stopped by itself only once a reply to it would have to read the whole chat
  again anyway, so resuming it costs what replying would have cost.
- **You have the last word**: a star or **Keep running** protects a chat for as long as you want.

## Use cases

- **A computer that stays on**: chats from last week don't pile up on the computer in the closet.
- **A long-running helper**: a chat that runs a server stays up because you marked it **Keep running**.
- **A chat that looks stuck**: a chat that has said "working" for half a day shows up for review, with
  **Stop** beside it.
- **A clean board**: you dismiss the chats you are done with, and they stay hidden until they are used again.

## How it works

Switchboard tidies up in two ways. One happens by itself, and one waits for you.

### Stopped by itself

A chat is stopped by itself when all of these are true:

- It runs in the background, not in a terminal that someone has open.
- It finished its reply and has been left alone for the wait you set. The default is 6 hours.
- A reply to it would read the whole chat again. Refer to [Warm and cold](../concepts/warm-and-cold.md).

A chat that was replaced by a newer chat, or archived in the Claude app, waits just as long.

Before anything is stopped, the home computer (the one computer that keeps the board) checks the chat, asks
the chat's own computer for a fresh report, and checks again. The chat's own computer then has the last word:
it stops the chat only if nothing would be cut off at that moment.

A finished chat that Claude started from another chat is stopped sooner, two minutes after its report. For more
information, refer to [Chats sent by Claude](chats-sent-by-claude.md).

### Asked about

For other chats, a strip under the top line says how many chats are open but not in use, and how much memory
they hold. Nothing on the strip stops until you click **Stop**. The strip lists:

- **Idle chats**: background chats that have done nothing for 3 days.
- **Stuck chats**: chats that say they are working, with nothing happening for 12 hours.
- **Replaced chats**: chats that a newer chat took over from, an hour or more ago.
- **Archived chats**: chats you archived in the Claude app, an hour or more ago.
- **Leftover processes**: Claude processes that no chat owns, 6 hours old or more.

### What is never stopped

Switchboard never stops or offers to stop a chat when:

- It is working.
- It is waiting for your answer, or a message you sent to it is held.
- You starred it or marked it **Keep running**.
- It is open in a terminal tab, even while the app is closed, or in a terminal on its own computer.

A chat is also never stopped by itself when:

- It runs a subagent, a background command or a scheduled task, or holds a result it hasn't delivered.
- Another chat is waiting for its report.
- It was resumed or restarted within the wait.
- Its computer is offline or can't say what is running. This is because Switchboard doesn't stop a chat on a
  guess.

A chat that looks stuck and a leftover process are only ever offered on the strip. They are never stopped by
themselves.

## Use tidy up

To keep your computers tidy:

1. Review the chats that are not in use.
2. Protect the chats you want to keep.
3. Stop or dismiss a chat yourself.
4. Bring a stopped chat back.

### Prerequisites

Before you can tidy up, you must:

- Have the chat's computer online. A chat on an offline computer can't be stopped or resumed.

### Review the chats that are not in use

1. Click **Review** on the strip that says "N chats are open but not in use."
2. Read each row. It names the chat, its computer and the reason, such as "Idle 4 days".
3. Click **Stop** on a row, then **Confirm stop**. The confirmation counts down for three seconds and then
   puts itself back. Press ++esc++ to cancel sooner.
4. Click **Keep** on a row to mark that chat **Keep running**.
5. Click **Stop all 3** (the button names how many) to stop every listed chat with one confirmation, or
   **Not now** to put the list away.

Chats you put off with **Not now** move under **Stopped today**, where **Show on the board** brings the strip
back.

### Protect a chat

To keep one chat running, right-click its card and select **Keep running**. The menu then reads
**Kept running**. Select it again to let Switchboard stop the chat when it is idle. **Keep running** is offered
only on a running background chat.

To star a chat, open its details and click the star beside its name, or right-click its terminal tab and select
**Star tab**. A starred chat is never stopped, its tab sits at the left of the tab row, and the tab comes back
when you open the app again.

### Stop or dismiss a chat yourself

Right-click a card and select one of:

- **Stop**: ends a running background chat. You can resume it later.
- **Dismiss**: hides the chat from the board until it is used again. On a running background chat it also
  stops the chat. On a chat that is open in a terminal on its own computer it only hides the chat, and the
  terminal stays open.

### Bring a stopped chat back

Click **Stopped today** at the right end of the top line. It lists every chat stopped in the last day, with how
long ago and how much memory was freed. Click **Resume** beside a chat.

A chat that was stopped by itself also stays in the **Quiet** column, with **Stopped** before its time. Select
it and press ++r++, or double-click it, to resume it. For what a resume costs, refer to
[Resume or start fresh](resume-or-start-fresh.md).

## Change the wait or turn it off

To change how long a finished chat is left alone before it is stopped by itself:

1. Open **Settings**.
2. Select **Chats**.
3. Under **Stop idle chats**, select **2 hours**, **6 hours** or **1 day**.

Select **Never** to turn it off. The setting is kept on the home computer, so it applies to every computer.
For more information, refer to [Settings](../settings.md).

## Limitations

- **Background chats only**: a chat that is open in a terminal on its own computer is never stopped. Close it
  in that terminal.
- **Two hours at the least**: the shortest wait is 2 hours. This is because a chat can still wake by itself, or
  be cheap to reply to, for about an hour after it goes idle.
- **A stopped chat isn't reachable from the Claude phone app**: resume it in Switchboard first.
- **Resuming reads the whole chat again**: by the time a chat is stopped, a reply to it would have done the
  same.
- **A few at a time**: Switchboard stops at most two chats a minute, so a long backlog takes a while to clear.

## Troubleshooting

### A chat I wanted was stopped

Resume it from **Stopped today** or from the **Quiet** column. Then mark it **Keep running** or star it so that
it isn't stopped again.

### An idle chat is never stopped

Check each of these in turn:

1. Open **Settings**, select **Chats** and check that **Stop idle chats** isn't set to **Never**.
2. Check that the chat isn't starred, marked **Keep running** or open in a terminal tab. A tab counts even
   while the app is closed.
3. Check that the chat isn't waiting on you in **Needs you**.
4. Check that the chat's computer is online. An offline computer's chats are left alone.

If a chat still runs a background command or a scheduled task, Switchboard leaves it alone and looks again
after 30 minutes.

### The strip keeps coming back

The strip returns while any listed chat is still open and not in use. Stop the chats, or click **Keep** on the
ones you want. **Not now** only puts the list away.
