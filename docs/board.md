---
description: The board shows every Claude Code chat on every computer, in a row for its computer and a column for what it needs from you.
---

# The board

<p class="sb-meta">Last validated: Nov 22, 2026</p>

The board lets you see every chat on every computer at once. Each computer gets a row, and each chat sits in a
column for what it needs from you: **Needs you**, **Working**, **Warm now**, **Gone cold** or **Quiet**.

## Why it matters

A chat that is waiting for you is rarely on the computer in front of you. The board puts that chat first, on
whichever computer it runs, so you don't have to check each terminal in turn.

## Benefits

- **One place to look**: every computer's chats are on one screen, and the ones that need you are in the first
  column.
- **Cost in view**: a card says whether a reply is cheap now, and what it would re-read once the chat has gone
  cold.
- **Answers without a trip**: a waiting chat shows what Claude asked, with a button to answer it from the card.
- **Old information looks old**: when a computer hasn't reported for 45 seconds, its chats turn gray and say
  how old their state is.

## Use cases

- **Check in after a break**: read the top line and the **Needs you** column to learn what stopped while you
  were away.
- **Reply while it is cheap**: the **Warm now** column lists the chat that is about to go cold first.
- **Decide what to do with a big cold chat**: a card in **Gone cold** says what resuming would cost and what a
  fresh start would cost.
- **Find yesterday's chat**: the **Quiet** column keeps each computer's newest idle chats, with a link to the
  rest in History.

## How it works

Each computer reports the state of its chats to one of your computers, known as the home computer. The home
computer decides each chat's state once, and the board, the terminal tabs and the phone apps all show that same
answer. For more information, refer to [Chat states](concepts/chat-states.md).

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/board-anatomy.svg"
<figcaption>Figure 1. Computers are rows and states are columns. The chat that needs you is in the first column,
whichever computer it runs on.</figcaption>
</figure>

1. A row is one computer. Its label shows the computer's system and how many chats it has.
2. A column is one state. The number beside each column's name counts the chats in it across every computer.
3. A card is one chat. It sits in its computer's row and in the column of its state.
4. As time passes without a reply from you or Claude, a chat moves to the right, from **Working** to
   **Warm now**, and then to **Gone cold** or **Quiet**.

The figure doesn't draw the top line above the grid or the **Sent by Claude** line under a row. Both are
described below.

### The columns

| Column | What puts a chat there |
|---|---|
| <span class="sb-pill need">Needs you</span> | Claude is waiting for your answer, or Switchboard is holding a message you sent to a big cold chat. |
| <span class="sb-pill run">Working</span> | Claude is working. A chat with no progress for 6 hours says **Stalled**. |
| <span class="sb-pill warm">Warm now</span> | The chat is idle and a reply is still cheap. The chat that goes cold soonest is first. |
| <span class="sb-pill cold">Gone cold</span> | The chat is idle, a reply would read the whole chat again, and the chat is big (150k tokens or more). |
| <span class="sb-pill">Quiet</span> | Everything else, one line each, newest first: small chats that have gone cold, chats whose cost isn't known yet, and stopped chats. |

A chat that has waited more than a day stays in **Needs you**, but it is muted, sorted to the bottom and no
longer counted. Its card says **Waiting 2 days**.

The **Quiet** column keeps each computer's 8 newest idle chats, however old they are. Older ones are in History,
and a link under the column says **N more in History**.

### A card

A card in the first four columns shows:

- **The name**: with a tag when the chat runs in a special mode, a dot in the colour of its tab group, and a
  star when you starred it.
- **A pill**: `warm`, `cold` or `cache ?` for what a reply costs, or `working`, `stalled` or `parked` when that
  says more.
- **One line for where it stands**: for example "Waiting 14m", "Message on hold", "Cache good for 4:12",
  "Resume 290k, fresh 91k" or "Working".
- **The size**: the chat's size in tokens, replaced by **expensive** on a chat of 150k tokens or more, or by
  **re-reads 312k** once that chat is cold.
- **The flags button**: at the right edge, with the number of launch flags the chat runs with. Click it to
  change them.

A card in **Needs you** also shows what Claude asked and one button to answer. For more information, refer to
[Answer from the board](features/answer-from-the-board.md).

A row in **Quiet** shows only the name, a `cold` pill for a small chat that has gone cold, and how long ago the
chat was last active. A chat that Switchboard stopped says **Stopped** before the time.

### The top line

Above the grid, one line tells you what to look at first. Switchboard picks the first of these that applies:

1. A chat that needs you, with **Open**.
2. A big chat that goes cold within 5 minutes, with **Reply now**.
3. Usage that is climbing much faster than usual.
4. A 5-hour window at 80% or more, or a week that is on pace to run out.
5. How the week is going.

The last three come with **Plan details**. When none applies, the line says "Nothing needs you right now."

At the right end of the top line, **Stopped today** lists the chats that were stopped today, each with
**Resume**. For more information, refer to [Tidy up](features/tidy-up.md).

### A computer's row

The label at the left of a row shows:

- **The computer's name**: with two buttons, one to fold the row and one to open the row in a window of its
  own.
- **Its system and chat count**: for example "macOS, 4 chats". A computer that can't be reached says
  **Offline**, and its chats come back when it does.
- **Memory**: how much memory Claude is using on that computer.
- **What it can do**: the tools the computer reports, when it reports any.
- **New terminal**: starts a new chat on that computer.

Under a row, the line **Sent by Claude** holds the chats that Claude started there from another chat. They stay
out of the columns. For more information, refer to [Chats sent by Claude](features/chats-sent-by-claude.md).

## Use the board

To work from the board:

1. Read the top line.
2. Open a chat's details.
3. Open a chat's terminal.
4. Use a chat's menu.
5. Fold a row or give it a window.

### Prerequisites

Before you can use the board, you must:

- Install Switchboard on your home computer. Refer to [Install Switchboard](install.md).
- Add at least one computer that runs Claude Code. Refer to [Add your computers](machines.md).

### Read the top line

Read the line above the grid, then click its button. **Open** selects the chat that needs you, **Reply now**
selects the chat that is about to go cold, and **Plan details** opens your usage.

### Open a chat's details

Click a card or a **Quiet** row. The details open in a sheet at the right, with the chat's state in a sentence,
what each next step would cost and every action.

### Open a chat's terminal

Double-click a card, or select it and press ++enter++. A running chat opens in a terminal tab. A stopped chat is
resumed first. For more information, refer to [Terminals](terminals.md).

### Use a chat's menu

Right-click a card or a **Quiet** row. The menu lists every action for that chat, with what the costly ones
would use:

- **Open terminal**
- **Start fresh**
- **Resume as is**
- **Continue on another machine**
- **Keep running**
- **Stop**
- **Dismiss**
- **Rename chat**

The menu leaves out an action that doesn't apply. For example, **Stop** appears only on a running background
chat.

### Fold a row or give it a window

Click the arrow beside a computer's name to fold its row. Click the window button beside it to open that
computer's row in a window of its own.

You can also move around the board from the keyboard. For more information, refer to
[Keyboard shortcuts](shortcuts.md).

## Limitations

- **Columns have a minimum width**: in a narrow window, or beside the details sheet, the grid scrolls sideways
  instead of squeezing the columns. This is because a card's name needs room to read as words.
- **Quiet is capped**: each computer shows its 8 newest idle chats. The rest are in History.
- **State can lag**: a computer that hasn't reported for 45 seconds shows its chats in gray with "As of 2m ago".
  The board shows the last known state until that computer reports again.
- **A card shows a short version of the question**: the full question and all of its controls are in the
  chat's details.

## Troubleshooting

### A card is gray and says "As of 2m ago"

The chat's computer hasn't reported for 45 seconds or more, or the app has lost its link to the home computer.
The state on the card is the last one known.

1. Check that the computer is on and awake.
2. Wait for the card to return to its usual colours. It does so as soon as the computer reports again.
3. If the whole board is gray, check that the home computer is running.

### A chat I expect is missing

1. Look in the **Quiet** column for the link **N more in History**, and open it.
2. Look under the computer's row for **Sent by Claude**. A chat that Claude started from another chat is
   listed there, not in the columns.
3. Check that the computer's row isn't folded.
