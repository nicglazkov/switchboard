---
description: Continue a chat on a different computer. Switchboard writes a summary where the chat is and starts a new chat from it on the computer you pick.
---

# Continue on another computer

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Continue on another computer lets you move a chat's work to a different computer. Switchboard writes a summary
of the chat on the computer it runs on, then starts a new chat from that summary on the computer you pick.

## Benefits

- **Change desks without starting over**: the new chat knows the goal, your instructions and what is done.
- **The chat's setup comes along**: model, effort, permission mode, agent, extra instructions and Chrome
  setting are kept.
- **The old chat stays**: it moves to **Replaced** and says where the work went.

## Use cases

- **From the laptop to the desktop**: you began a task on the couch and want the faster computer to finish it.
- **To the computer with the right tools**: the next step needs Xcode, a graphics card or Windows.
- **Before a computer goes to sleep**: move a long task to the computer that stays on.

## How it works

Continue is [Start fresh](resume-or-start-fresh.md) with a different destination:

1. Switchboard asks the chat's computer to write a summary of the chat.
2. It sends the summary to the other computer and starts a new chat there with it.
3. The old chat is marked **Continued on** with the other computer's name, and moves to **Replaced**.

Only the summary travels. The chat's history and your files stay on the first computer. This is because each
chat works with the files of the computer it runs on.

The new chat starts in the other computer's Claude folder, the folder Switchboard keeps chats in by default.
When Claude moves a chat itself, as described below, it starts in the folder of the same name on the other
computer if that computer has it.

What comes along with the chat:

- **Model and effort**: the same as the old chat.
- **Permission mode, agent and extra instructions**: the same, when the other computer's Claude Code takes them.
- **Claude in Chrome**: on, if the old chat had it.
- **Extra folders**: each one that also exists on the other computer.
- **Tools from your library**: installed on the other computer first if it lacks them.

A setting the other computer can't take is left out. Leaving one out never stops the move.

## Continue a chat on another computer

To move a chat:

1. Select the chat.
2. Choose the computer to continue on.
3. Open the new chat when it is ready.

### Prerequisites

Before you can continue a chat on another computer, you must:

- Have at least two computers on your board, both online. Refer to [Add your computers](../machines.md).
- Have the project's files on the other computer if the work needs them.

### Select the chat

Select the chat on [the board](../board.md) and press ++c++, or open the chat's menu and choose **Continue on
another machine**. With no other computer online, the action is greyed out.

### Choose the computer

Choose a computer from the list. The chat's menu also lists each one directly, as **Continue on** followed by
the computer's name.

If Claude is still replying, Switchboard asks whether to move the chat when Claude finishes, or now, which
stops the reply.

### Open the new chat

Writing the summary takes about a minute. The new chat then appears on the board under the other computer. The
old chat says **Continued on** and the computer's name.

## Let Claude move the chat

You can also ask Claude, in the chat, to continue on another computer. Every chat on a computer that runs
Switchboard has a tool for it. Name the computer as the board shows it:

```text
Continue this on example-mac.
```

Replace `example-mac` with your computer's name.

## Limitations

- **Files don't move.** Uncommitted changes on the first computer are not on the second. Push or copy them
  first.
- **The new chat starts from a summary.** It doesn't have the old chat's full history. Refer to
  [Resume or start fresh](resume-or-start-fresh.md).
- **A worktree doesn't come along.** A worktree is set when a chat starts and belongs to one computer.
- **Both computers must be online** until the new chat has started.
