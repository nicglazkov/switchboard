---
description: Start a Claude Code chat on any of your computers from New chat. Pick the computer or let Auto pick, then the folder, model, effort, mode, permissions and tools.
---

# Start a chat

<p class="sb-meta">Last validated: Nov 22, 2026</p>

**New chat** lets you start a Claude Code chat on any of your computers without sitting at it. You pick the
computer and the folder, or let Switchboard pick the computer, and say what the chat should work on.

The chat runs in the background on that computer. It shows on the board at once, and you can open its terminal
from any device.

## Benefits

- **Any computer from one place**: start work on the computer in the closet from the laptop on the couch.
- **Auto finds the right computer**: Switchboard sends an iOS task to the computer with Xcode.
- **The same choices as the command line**: model, effort, permissions, extra folders and other Claude Code
  flags.
- **Saved setups**: keep a combination you use often and bring it back in one click.

## Use cases

- **A quick task on another computer**: type one message, pick the computer and go back to what you were doing.
- **A big build**: start it in **UltraCode** mode on the computer with the most memory.
- **A second attempt in isolation**: start a chat in its own worktree so it can't disturb the first.
- **A task that needs a browser**: turn on **Claude in Chrome** and let Auto send it to your Chrome computer.

## How it works

Switchboard sends your choices to the helper on the computer you picked. The helper starts Claude Code there,
in the folder you chose, with your first message. Nothing runs on the device you are holding.

When you pick **Auto** instead of a computer, Switchboard compares the computers that are online and picks one
for you.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/start-auto.svg"
<figcaption>Figure 1. Auto reads the first message for what the task needs, then picks the computer that has
it.</figcaption>
</figure>

Auto goes by these rules, in order:

1. **What the task needs**: words in your message point to a tool. iOS words need Xcode, Android words need
   the Android SDK (software development kit), training and GPU (graphics card) words need CUDA, and Windows
   words need Windows.
2. **A connected phone**: among computers with the right tool, one with your iPhone or Android device connected
   comes first.
3. **The repository**: a computer that has the repository your message names.
4. **Free memory**: otherwise, the computer with the most free memory.

A chat that uses Claude in Chrome goes to its Chrome computer. Only computers that share what they can do take
part in Auto. You choose which ones in **Settings**, under **Machines**. Refer to
[Settings](../settings.md#machines).

With **Let Claude pick for Auto** turned on in Settings, Claude makes the pick from your message and each
computer's tools, free memory and repository names. If Claude can't answer, the rules pick. This option uses
your plan and sends the message to Claude.

## Start a chat

To start a chat:

1. Open **New chat**.
2. Pick the computer and the folder.
3. Pick the mode and the model.
4. Set the options you need.
5. Write the first message and select **Start**.

### Prerequisites

Before you can start a chat, you must:

- Have Switchboard set up. Refer to [Install Switchboard](../install.md).
- Have at least one computer online. Refer to [Add your computers](../machines.md).

### Open New chat

Press ++n++ on the board, or ++cmd+n++ on a Mac and ++ctrl+n++ on Windows or Linux from anywhere in the app.
The command palette has **New chat** too.

### Pick the computer and the folder

Under **Machine**, pick a computer that is online, or **Auto**.

Under **Folder**, pick where Claude works. The recent folders of that computer are one click away.
**Choose…** opens that computer's folders so you can browse to any of them, and typing a name in **New
folder** makes one. With **Auto**, the folder is optional: type a repository's name and Auto finds the computer
that has it.

For more information, refer to [Places and folders](../concepts/places-and-folders.md).

### Pick the mode and the model

**Mode** has three choices:

- **Normal**: your usual effort.
- **Max effort**: the deepest reasoning on every turn.
- **UltraCode**: multi-agent workflows for big builds, on Opus at extra high effort. It uses your plan faster.

**Model** is **Default** or one of the models your computers' Claude Code offers. **Default** leaves the
choice to Claude Code on that computer.

### Set the options

Each option can stay as it is. They are:

| Option | Choices |
|---|---|
| **Browser** | **Claude in Chrome**, on or off |
| **Tools** | **Auto**, or **Pick** to choose tools from your library |
| **Permissions** | **Machine default**, **Asks first**, **Accept edits**, **Plan**, **Auto**, **Bypass**, **Never asks** |
| **Effort** | **Default**, **Low**, **Medium**, **High**, **Extra high** |
| **Folders** | Up to 8 more folders Claude may also use |
| **Other flags** | More Claude Code flags, typed as on the command line |

**Machine default** starts the chat in whatever mode that computer's own Claude Code settings say, which may
not be one that asks first. **Asks first** makes Claude ask before each change.

**Max effort** and **UltraCode** set the effort themselves, so **Effort** is locked while one is picked.

Extra folders belong to the computer they were picked on. Changing the computer clears them.

### Start the chat in a worktree

A worktree is a separate copy of a repository's files, so two chats can work on the same project without
touching each other's changes. To start the chat in one, type the flag under **Other flags**:

```text
--worktree example-fix
```

Replace `example-fix` with a name for the worktree, or leave the name out and Claude Code picks one. A worktree
is set when a chat starts and can't be changed afterwards.

### Write the first message and start

Under **First message**, say what the chat should work on. Leave it empty to start a chat that waits for you.
Leave **Open its terminal** checked to watch it start, then select **Start**.

Switchboard keeps an unsent first message as a draft for a week.

## Save a setup

A setup is a named combination of these choices. To save one, set the choices, select **Save setup**, name it
and select **Save**.

Above the form, **Defaults** clears every choice, **Last used** brings back what you started the last chat
with, and each saved setup has its own button. To remove a setup, open **Settings**, then **Chats**, and find
it under **Saved setups**.

## Start in a folder Claude hasn't worked in

Before Claude works in a folder outside the Claude folder for the first time, Switchboard asks **Let Claude
work in** followed by the folder's name. This is the same question Claude Code asks in a terminal.

If you say yes, Claude can read and change the files in that folder and the folders inside it, and run
commands there. If you select **Not now**, the chat isn't started.

New chat tells you ahead of time. Under the folder it says **Claude hasn't worked here yet. Start asks you
first.**

## Limitations

- **The computer must be online.** A chat can't be queued for a computer that is asleep or off.
- **Auto needs something to go by.** With an empty first message it falls back to free memory.
- **Some flags aren't passed on.** **Other flags** takes the flags Switchboard knows how to pass to Claude
  Code, and says so when a flag isn't one of them.
- **A computer with an older Switchboard may ignore Asks first.** It starts the chat in its own default mode
  instead.

## Troubleshooting

### Auto says no machine is online

New chat says **No machine is online for Auto to pick** when none of the computers Auto may use is online. The
server answers **Auto is off: no machine shares what it can do yet** when no computer shares what it can do.
Pick a computer by name, or turn sharing on in **Settings**, under **Machines**.

### The chat didn't start

New chat stays open and says **Didn't start** with the reason from that computer. The usual causes are a
folder that no longer exists and a flag that computer's Claude Code doesn't take.
