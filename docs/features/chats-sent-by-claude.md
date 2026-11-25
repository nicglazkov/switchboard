---
description: A chat can send a task to another computer, where a new chat does it and reports back. Switchboard marks those chats Sent by Claude and keeps them apart from your own.
---

# Chats sent by Claude

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Chats sent by Claude lets you tell your own chats apart from the ones Claude started. When a chat sends a task
to another computer, a new chat does the work there and reports back. Switchboard marks that new chat
**Sent by Claude** and keeps it out of the board's columns.

## Why it matters

One chat can send several tasks, and each becomes a chat of its own. If they sat in the columns with your
chats, the board would fill with work that isn't waiting for you. A question in a task's report is for the
chat that sent it to read, not for you.

## Benefits

- **Your columns stay yours**: a sent chat doesn't sit in a column or count in a column's number, unless it
  really is waiting for you.
- **You can still follow the work**: each computer's row lists its sent chats, with where each one stands and
  which chat sent it.
- **It cleans up after itself**: a finished task's process is stopped two minutes after its report, and the
  chat leaves the board soon after.
- **Nothing is lost**: the report is kept, the chat is in History, and **Resume** brings it back.

## Use cases

- **Work that needs another computer**: a chat on your laptop sends a Windows build to the computer that has
  the tools for it.
- **Several tasks at once**: one chat sends three tasks to three computers and gathers the reports.
- **A task worth keeping**: a sent chat turns into a longer piece of work, so you move it to your own chats.

## How it works

Claude sends a task with Switchboard's `run_on` tool. The task names a computer, or `auto` to let Switchboard
pick one. A new background chat starts on that computer, does the task and ends with a short report.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/sent-task.svg"
<figcaption>Figure 1. The task runs as a new chat on the other computer, listed under Sent by Claude. Its report
returns to the chat that asked.</figcaption>
</figure>

1. Your chat sends the task. A new chat starts on the other computer and appears under that computer's row, in
   the line **Sent by Claude**. The new chat sees nothing of your chat, only the task.
2. The new chat finishes and its report goes back to your chat, which reads it and carries on.

The chat that sent the task shows a count in its details, such as "1 task out: 1 working".

### Where a sent chat shows

A sent chat is listed under the row of the computer it runs on, not under the chat that sent it. Each row ends
with one line, **Sent by Claude**, followed by how many there are, how many are working and how many have
finished.

Click the line to open it. Each sent chat has a row with:

- **Its name**: click it to open the chat's details, or double-click it to open its terminal.
- **Where it stands**: one of the lines in the table below.
- **Who sent it**: for example "from Ship the widget on Laptop". Click it to go to that chat.
- **Stop or Clear**: **Stop** while it runs, **Clear** once it has stopped.

Working chats come first, then finished ones, newest first.

| Line | What it means |
|---|---|
| Working | The chat is doing the task. |
| Stalled, nothing for 7h | It says it is working, but nothing has happened for 6 hours or more. |
| Waiting for you 12m | It is stopped at a prompt that only you can answer. |
| Message on hold | A message you sent to it is held. |
| Done 2h ago, report read | It finished, and the chat that sent it has read the report. |
| Done 2h ago, report not read yet | It finished, and nobody has read the report. |
| Stopped without a report, 2h ago | It ended before it reported. |

### When a sent chat needs you

A sent chat reports to the chat that sent it, so a question at the end of its report is not yours to answer.
It moves to **Needs you** only while it is stopped at a prompt that only you can answer, such as a permission
question, or while a message you sent to it is held.

In **Needs you** its card carries the **Sent by Claude** mark and the line that says who sent it. The top line
and the notification say that a task Claude sent is waiting.

### How a sent chat leaves

- **Its process stops**: two minutes after the report, Switchboard stops the chat's process to free its
  memory.
- **It leaves the board**: a finished chat moves to History 10 minutes after its report is read, or after a
  day when nobody read it.

Switchboard doesn't stop a finished chat's process when you have a use for it. That is the case when:

- The chat is open in a terminal or a tab.
- You marked it **Keep running**.
- You moved it to your own chats.
- It is working again, or was used after it reported.
- It is waiting at a prompt, or a message to it is held.

## Use chats sent by Claude

To work with a sent chat:

1. Open the Sent by Claude line.
2. Stop or clear a sent chat.
3. Move a sent chat to your own chats.
4. Find a sent chat in History.

### Prerequisites

Before a chat can send a task, you must:

- Have another computer on the board, and have it online. Refer to [Add your computers](../machines.md).
- Allow the request when Claude Code asks. Sending a task starts work on another computer, so Claude Code
  asks for permission first unless your permission settings already allow it.

### Open the Sent by Claude line

Click **Sent by Claude** under a computer's row. The line opens into one row for each sent chat. Click it again
to fold it.

### Stop or clear a sent chat

Click **Stop** on a running chat's row, then click **Confirm stop**.

Click **Clear** on a stopped chat's row to take it off the board at once. A chat that is open in a terminal on
its own computer has neither button, because it isn't Switchboard's to stop.

### Move a sent chat to your own chats

Right-click the sent chat, or open its details, and select **Move to my chats**. From then on it sits in the
columns and counts like any of your chats, and Switchboard no longer stops it two minutes after a report.

### Find a sent chat in History

Open History and set the filter that reads **All chats** to **Sent by Claude**. Select **Your chats** to hide
them. For more information, refer to [History](history.md).

## Limitations

- **Listed by computer, not by sender**: a sent chat appears under the computer it runs on. The chat that sent
  it shows only a count of its tasks.
- **A stopped chat's report stays, its process doesn't**: to talk to a finished sent chat, resume it first.
  Resuming re-reads the whole chat. For more information, refer to
  [Resume or start fresh](resume-or-start-fresh.md).
- **A question in a report doesn't reach you**: the sending chat reads it. If that chat needs your answer, it
  asks you itself.
- **No list on an offline or folded row**: the line shows only while its computer's row is open and the
  computer is online.

## Troubleshooting

### A sent chat disappeared from the board

It finished, and its time on the board ran out: 10 minutes after its report was read, or a day when nobody read
it. Open History and filter by **Sent by Claude** to find it.

### A sent chat is in Needs you

It is stopped at a prompt that only you can answer, usually a permission question. Double-click its card to
open its terminal and answer there. For more information, refer to
[Answer from the board](answer-from-the-board.md).

### A sent chat was stopped while I was reading it

Switchboard stops a finished sent chat two minutes after its report unless the chat is open in a tab or marked
**Keep running**. Open the chat in a tab, select **Keep running** from its menu, or select **Move to my chats**.
