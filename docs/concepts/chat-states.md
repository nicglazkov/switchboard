---
description: Every state a Claude Code chat can be in on the Switchboard board, what moves a chat from one state to the next, and how Switchboard knows.
---

# Chat states

<p class="sb-meta">Last validated: Nov 22, 2026</p>

A chat's state is the one answer to two questions: is the chat doing something, and what would a reply to it
cost right now. Switchboard works that state out once for each chat, and the board, the terminal tabs and the
phone apps all show the same answer.

There are five states, and each is a column on the board: **Needs you**, **Working**, **Warm now**,
**Gone cold** and **Quiet**. A few of them have a special case, such as a held reply or a stopped chat, which
this page describes under the state it belongs to.

## Needs you

A chat is in <span class="sb-pill need">Needs you</span> when Claude has stopped and is waiting for your answer.
That can be a question at the end of a reply, a question Claude asks in the middle of its work, or a request
for permission to use a tool.

A chat that has waited more than a day is parked. It stays in the column, but it is muted, sorted to the
bottom, and no longer counted or announced. Answering it works as usual.

## Reply held

A held reply is a special case of **Needs you**. When you send a message to a big chat that has gone cold,
Switchboard holds the message before Claude reads it, because that one reply would read the whole chat again.
The card says "Message on hold".

The chat stays there until you decide: send the message anyway, or start fresh in a new chat that carries the
message with it. A hold that nobody answers lapses after 6 hours. For more information, refer to
[Cold chat guard](../features/cold-chat-guard.md).

## Working

A chat is <span class="sb-pill run">Working</span> while Claude is replying or running tools. A background chat
whose reply has ended still counts as working while its subagents or background commands are writing output.

A chat that says it is working but has made no progress for 6 hours is stalled. It stays in the column and its
card says **Stalled**, so that a stuck chat doesn't look busy.

## Warm now

When a reply ends, the chat is <span class="sb-pill warm">Warm now</span>. Claude keeps a recently read chat
ready for a short time, and while that lasts a reply reads only what is new. The card counts the time down, for
example "Cache good for 4:12".

For more information, refer to [Warm and cold](warm-and-cold.md).

## Gone cold

When the time runs out, a reply has to read the whole chat again. A chat of 150k tokens or more then moves to
<span class="sb-pill cold">Gone cold</span>, because that reply is expensive. Its card shows what resuming
would cost beside what a fresh start would cost.

## Quiet

Everything else is <span class="sb-pill">Quiet</span>. That covers three kinds of chat:

- **Small chats that have gone cold**: reading a small chat again is cheap, so it doesn't need the
  **Gone cold** column. Its row carries a `cold` pill.
- **Chats whose cost isn't known yet**: Switchboard shows `cache ?` instead of guessing.
- **Stopped chats**: chats whose process has ended.

## Stopped

A stopped chat isn't running, but nothing of it is lost. Its history is kept and **Resume** brings it back. A
chat can be stopped in three ways:

- **Stopped to free memory**: Switchboard stopped an idle chat. Its row says **Stopped** before the time, and
  its details say how long it had been idle. For more information, refer to
  [Tidy up](../features/tidy-up.md).
- **Stopped by you**: you chose **Stop** or closed the chat.
- **Stopped by a restart**: the chat's computer restarted, for example after an update or a power cut, and
  the chat ended with it. If the chat had a tab, the tab comes back and says "Stopped when" followed by the
  computer's name and "restarted".

Switchboard tells a restart apart from a stop by comparing two times. A chat that isn't running, and whose last
activity came before its computer last started, was ended by the restart.

## What moves a chat

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/states-flow.svg"
<figcaption>Figure 1. Time moves a chat to the right. A reply from you moves it back to Working, except for a big
cold chat, whose reply is held first.</figcaption>
</figure>

Two things move a chat to the right without anyone acting: a reply ends, and then the cache runs out. Where a
chat lands after that depends only on its size.

Everything else is an action. Claude asking moves a chat to **Needs you**. Your answer, your reply or a resume
moves it back to **Working**. The one exception is the held reply: a message to a chat in **Gone cold** moves
the chat to **Needs you** first, and to **Working** only once you send the message anyway.

A chat leaves the board when it has gone unused long enough: 2 days for a chat that nothing has open, or 7 days
for one that is still open. The **Quiet** column still keeps each computer's 8 newest chats, however old. The
rest are in History.

## How Switchboard knows

Switchboard doesn't watch the screen or read what you type. A small helper on each computer reads what Claude
Code already records there:

- **Claude Code's own list of chats**: whether each chat is busy, idle, waiting or stopped.
- **Each chat's history file**: when the chat last said something, and how big it is.
- **Claude Code's signals**: Claude Code tells the helper the moment a message is sent, a reply ends or a
  permission is asked for. It also reports when the chat's cache runs out.

The helper reports to one of your computers, known as the home computer, which asks every computer again every
few seconds. A signal from Claude Code shows on the board within a second or two, without waiting for the next
round.

The home computer then decides the state in a fixed order. A held message comes first, then waiting, then
working, and a chat that is none of these is quiet. For a quiet chat, the cache and the size choose between
**Warm now**, **Gone cold** and **Quiet**. Because one computer decides, two apps never disagree about a chat.

!!! note ""
    When a computer hasn't reported for 45 seconds, its chats turn gray and say how old their state is, such as
    "As of 2m ago". The state shown is the last one known, not a guess at the current one.
