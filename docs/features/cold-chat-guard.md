---
description: The guard holds a message you type into a big chat whose cache has gone cold, shows what sending it would cost, and lets you send it anyway or start fresh.
---

# The cold chat guard

<p class="sb-meta">Last validated: Nov 22, 2026</p>

The guard holds a message you type into a big chat that has gone cold. It shows what sending the message would
cost and lets you choose: send it anyway, or carry it into a new chat that starts from a summary.

A chat is cold when Claude no longer keeps a ready copy of it, so the next message reads the whole chat again.
For more information, refer to [Warm and cold](../concepts/warm-and-cold.md).

## Why it matters

Nothing in Claude Code tells you that a chat went cold while you were away. You type one line into a chat of
250k tokens and that line costs as much as a long working session. The guard stops that message once and asks.

## Benefits

- **No expensive surprise**: you see the cost before the message is sent, not after.
- **Your message is kept**: whichever way you choose, the message you typed goes through without retyping.
- **It asks once**: after you send the message anyway, the chat is warm again and the guard leaves it alone.
- **Unattended work is never stalled**: prompts that nobody typed pass straight through.

## How it works

The guard runs on the chat's own computer, inside Claude Code, each time a message is sent. It holds a message
only when both of these are true:

- The chat holds 150k tokens or more.
- The chat's cache has run out. When Switchboard can't tell, an hour without a reply counts as cold.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/guard-flow.svg"
<figcaption>Figure 1. The guard answers a message to a big cold chat with a choice. Either way on, your message
is delivered.</figcaption>
</figure>

1. You send a message to a chat of 150k tokens or more whose cache has gone cold.
2. The guard holds the message. It answers in the chat with the size of the chat, how long it has been idle,
   and what **Resume** and **Start fresh** would each cost. On the board the chat moves to
   <span class="sb-pill need">Needs you</span> with **Message on hold**.
3. If you choose **Send anyway**, the message goes to the same chat. Claude reads the whole chat again once,
   and the chat is warm from then on.
4. If you choose **Start fresh** instead, Switchboard writes a summary of the chat and opens a new chat with
   the summary and your held message.

The figure doesn't show notifications. If they are set up, your iPhone gets one **Message held** notification
for each held message. Refer to [Notifications](notifications.md).

### What is held and what never is

| Message | Held? |
|---|---|
| A message you type | Yes |
| A message that starts with text you pasted | Yes |
| A command that starts with `/` | No |
| A prompt fired by a schedule or a loop | No |
| A background task or subagent reporting back | No |
| A message from another chat or agent | No |
| Any message to a warm chat, or to a chat under 150k tokens | No |

Prompts that nobody typed are let through because nobody is there to answer the guard. Holding one would stall
the work until you came back.

A chat whose subagents or background commands wrote something within the last hour counts as working, not
cold, and its messages aren't held.

## Answer a held message

You can answer from the chat's terminal or from the board.

### Answer in the terminal

To send the message anyway, send the same message again, or type `go`. Either works for 15 minutes after the
hold.

```text
go
```

To start fresh, type `/clear`, then send anything. Switchboard writes the summary, which takes about a minute,
loads it in the new chat and carries your held message over.

```text
/clear
```

### Answer from the board

Open the chat's details on [the board](../board.md). They offer two actions:

- **Send anyway**: sends the held message to the same chat.
- **Start fresh**: opens a new chat with your notes and the held message.

For what Start fresh does, refer to [Resume or start fresh](resume-or-start-fresh.md).

## Turn off the notification

The guard itself has no switch. To stop its notifications, open **Settings**, then **Notifications**, and turn
off **Held messages**.

## Limitations

- **It only sees chats on computers that run Switchboard.** A chat in the Claude apps is never held.
- **A hold doesn't wait forever.** After 6 hours without an answer the board stops showing the chat as held.
  The message was never sent, so type it again.
- **The costs it shows are estimates.** Refer to [What a chat costs](../concepts/what-a-chat-costs.md).
- **A scheduled prompt can wake a cold chat at full price.** This is because the guard never holds what nobody
  is there to answer.

## Troubleshooting

### My message was held but the chat feels recent

The cache lasts 5 minutes or 1 hour after the last reply, whichever Claude Code gave that chat. A chat you
answered ten minutes ago can already be cold. Send the same message again to go on.

### I typed go and nothing was sent

`go` and sending the same message again work for 15 minutes after the hold. After that, `go` is treated as a
new message and is held itself. Type your original message again. It is held once more, and `go` then sends it.
You can also choose **Send anyway** in the chat's details.
