---
description: Answer a waiting Claude Code chat from its card on the Switchboard board, without going to the computer it runs on.
---

# Answer from the board

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    This page covers the desktop app. Sending an answer needs the chat's computer to be online.

Answer from the board lets you reply to a waiting chat from its card. The card in **Needs you** shows what
Claude asked, shaped as a choice, a short form, a checklist or a box to type in, and Switchboard types your
answer into the chat for you.

## Why it matters

A long task often stops on a small question: which of two options, or whether a step is done. Without an
answer, the chat waits, on a computer you may not be sitting at. One click from the board sets it going again.

## Benefits

- **One click for a clear choice**: when Claude recommends an option, the card's button sends it.
- **The question in plain sight**: the card shows what Claude needs on two lines, so you don't have to open the
  chat to learn why it stopped.
- **Nothing guessed about you**: a question about you, such as what you own or prefer, never comes with an
  answer picked in advance.
- **Your words, typed for you**: the answer goes into the chat's own terminal, the same as if you had typed it
  there.

## Use cases

- **Pick an option**: Claude offers two designs and recommends one. You click the recommended one on the card.
- **Confirm a manual step**: Claude asks you to upload a file somewhere. You do it, then click **Done**.
- **Answer several questions at once**: Claude asks three things. You pick a chip for each and send them
  together.
- **Correct an assumption**: Claude lists what it assumed. You untick the one that is wrong before you send.

## How it works

A small helper on each computer watches that computer's chats and reports to one of your computers, known as
the home computer. The home computer keeps the board.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/answer-flow.svg"
<figcaption>Figure 1. The question travels out to you along the top. Your answer travels back to the same chat
along the bottom.</figcaption>
</figure>

1. The chat stops and asks. Claude Code records that it is waiting, and the helper on that computer notices
   within a few seconds.
2. The helper reports the chat's new state and the question to the home computer.
3. The board moves the chat to **Needs you** and shows the question on its card. For how you are told when the
   app isn't in front of you, refer to [Notifications](notifications.md).
4. You pick an answer on the card or in the chat's details.
5. The app opens the chat's terminal in a tab, types your answer and presses ++enter++. The chat returns to
   **Working**.

The figure doesn't draw the path the terminal takes between your computer and the chat's computer. For more
information, refer to [Terminals](../terminals.md).

### What a card shows

A card in **Needs you** shows how long the chat has waited, a tag for the kind of question, Claude's summary
on at most two lines, and one button.

| Tag | What the chat's details offer | The card's button |
|---|---|---|
| **Choose** | Claude's options, with **Claude recommends** on the one it prefers, and its assumptions to untick | The recommended option, when there is one and there are no assumptions to check. Otherwise **Answer...** |
| **Questions** | One row of choices for each question, and a field to type your own | **Answer...** |
| **Do this, then confirm** | Numbered steps to tick off, **Done** and **Hit a problem...** | **Done** |
| **Fix setup** | A button that fixes the problem, such as **Restart this chat with Chrome** | The same fix |
| **Open question** | A box to type your answer | **Answer...** |

**Answer...** doesn't send anything. It opens the chat's details, where the full controls are.

### Where the question comes from

Switchboard builds the card from the first of these that applies:

1. **Claude Code's own question**: when Claude asked with its question tool, the card shows those questions and
   options exactly.
2. **A summary by Claude**: with **Summarize waiting chats** turned on, Claude reads the chat's last message and
   shapes it into one of the five kinds. This uses your Claude plan and sends that last message to Claude.
3. **The chat's own words**: with the setting off, the card is an **Open question** that shows what the chat
   says it needs.

**Summarize waiting chats** is off on a new installation.

## Use answer from the board

To answer a waiting chat:

1. Find the chat in **Needs you**.
2. Answer with the card's button, or open the details.
3. Check that the answer went in.

### Prerequisites

Before you can answer from the board, you must:

- Have the chat's computer online. An offline computer's card says that you can't send answers right now.
- Have a chat that Switchboard can open in a terminal tab. A chat that is open in a terminal on its own
  computer can't be opened in a tab, so answer it at that terminal. For more information, refer to
  [Terminals](../terminals.md).

### Find the chat in Needs you

Open the board and look at the first column. The top line also names a chat that needs you, with an **Open**
button that selects it.

### Answer with the card's button

Click the button on the card when it names the answer you want: the recommended option, **Done**, or the fix.
Switchboard sends it at once.

A fix asks you to confirm first. **Restart this chat with Chrome** shows what the restart would re-read, with
**Restart with Chrome** and **Cancel**.

### Answer in the details

Click **Answer...**, or click the card itself, to open the chat's details. Then:

=== "Choose"

    1. Click the option you want.
    2. Untick any of Claude's assumptions that are wrong.
    3. Type a note in **Anything to change? (optional)** if you have one.
    4. Click **Send**.

=== "Questions"

    1. For each question, click a choice or type your own answer.
    2. Click **Send answer**. The button stays off until every question has an answer.

=== "Do this, then confirm"

    1. Do the steps. You can tick each one off as you go.
    2. Click **Done**.
    3. If a step fails, click **Hit a problem...**, type what went wrong and click **Send**.

=== "Open question"

    1. Type your answer in the box.
    2. Click **Send**.

### Check that the answer went in

The chat's terminal opens in a tab and your answer appears in it. Under the card, a line says "Sent:" followed by
what was typed. The chat then moves from **Needs you** to **Working**.

### Answer a permission question

When Claude Code stops to ask permission for a tool, the chat moves to **Needs you** like any waiting chat.
The permission question is Claude Code's own menu, not a message, so answer it in the chat's terminal:

1. Double-click the card to open the chat's terminal.
2. Choose an option in Claude Code's prompt.

## Turn the summary on or off

To choose whether Claude sums up what a waiting chat asks:

1. Open **Settings**.
2. Select **AI and your Claude setup**.
3. Turn **Summarize waiting chats** on or off.

With it off, a card shows the chat's own words and the kind is **Open question**, unless Claude asked with its
question tool. For more information, refer to [Settings](../settings.md).

## Limitations

- **The answer is typed, not delivered another way**: Switchboard opens the chat's terminal and types into it.
  If the terminal can't open, nothing is sent.
- **A card is cut short**: the summary takes two lines and an option's label takes one. The details show all
  of it.
- **Limits on what a card holds**: a choice shows up to 4 options and 6 assumptions, a form up to 5 questions
  with up to 5 choices each, and a checklist up to 8 steps.
- **One fix**: the only setup problem the app can fix for you is a chat that needs Claude in Chrome and was
  started without it.
- **A summary waits for the chat to settle**: Claude writes it only after the chat has been still for 20
  seconds. Until then the card shows the chat's own words.
- **A chat that has waited more than a day is muted**: it stays in **Needs you** and you can still answer it,
  but it no longer counts toward the column's number. For more information, refer to
  [Chat states](../concepts/chat-states.md).

## Troubleshooting

### "Not sent, its terminal didn't open. Try Send again."

The app waited for the chat's terminal and it never became ready, so nothing was typed.

1. Check that the chat's computer is online on the board.
2. Click **Send** again.
3. If it fails again, double-click the card to open the terminal yourself, and type the answer there.

### The card says the computer is offline

The chat's computer can't be reached, so there is no terminal to type into. The card keeps the question. Send
your answer when the computer is back on the board.

### The card shows the chat's words, not a choice

Switchboard shapes a question into a choice or a form only when Claude asked with its question tool, or when
**Summarize waiting chats** is on. Turn the setting on, or type your answer in the box.
