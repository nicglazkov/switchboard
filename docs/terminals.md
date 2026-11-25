---
description: Open any Claude Code chat's terminal in the Switchboard app, whichever computer it runs on, next to your own shells, with tabs, groups, copy and paste, and files you drag in.
---

# Terminals

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    A chat's terminal needs a Mac or Linux home computer. Your own shells work with any home computer.

Terminals let you open any chat in the Switchboard app and type into it, whichever of your computers it runs on.
The same tab row also holds your own shells, which run on the computer you're at.

## Why it matters

A Claude Code chat lives in a terminal on one computer. Without Switchboard you have to sit at that computer, or
connect to it yourself, before you can read what Claude wrote or type a reply.

## Benefits

- **Every chat, one window**: a chat on the computer in the closet opens in a tab as quickly as one on the computer
  you're at.
- **Nothing to set up per computer**: once a computer is on the board, its chats open. You don't write any
  connection commands yourself.
- **Closing isn't stopping**: closing a tab only leaves the chat. The chat keeps running on its computer, and you
  can open it again from anywhere.
- **It survives a bad connection**: a terminal that loses its connection comes back by itself, with the screen and
  the scrollback you had.

## Use cases

- **Answer in full**: a question needs more than a short reply from the board, so you open the chat and work in it.
- **Watch a long task**: keep a working chat in its own window on a second screen.
- **Keep related chats together**: group the tabs of one project, give the group a name and a color, and fold it
  when you switch to something else.
- **Run a quick command**: open your own shell beside your chats without leaving the app.

## How it works

A chat keeps running on its own computer. The tab in the app is a view of it: what you type travels to the chat,
and what the chat draws travels back.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/term-keystroke.svg"
<figcaption>Figure 1. Keys go out and the screen comes back along the same path, through your home
computer.</figcaption>
</figure>

1. You press a key in a terminal tab. The app sends it to one of your computers, known as the home computer,
   which is the one that runs Switchboard.
2. The home computer passes the key to the chat's computer, where Claude Code receives it as if you had typed it
   there.
3. The chat's screen changes, and its computer sends the change to the home computer.
4. The home computer passes it to the app, and the tab redraws.

When the chat runs on the home computer itself, steps 2 and 3 happen inside that one computer. The figure doesn't
draw how the computers reach each other. For more information, refer to
[How the pieces fit](architecture.md).

Your own shells take no part in this. A shell runs on the computer you're at and never goes through the home
computer. It isn't on the board, and it isn't reopened the next time you start the app.

## Use terminals

1. Open a chat's terminal.
2. Open your own shell when you need one.
3. Arrange and group the tabs.
4. Copy, paste and drag files in.

On Windows and Linux, press ++ctrl++ where this page says ++cmd++. While a terminal has the keyboard there, add
++shift++ as well (++ctrl+shift+t++), because plain ++ctrl++ keys belong to the program in the terminal. Refer to
[Keyboard shortcuts](shortcuts.md#in-a-terminal).

### Prerequisites

Before you can open a chat's terminal, you must:

- Have the chat's computer on the board and online. Refer to [Add your computers](machines.md).
- Run Switchboard on a Mac or Linux home computer, or on a Windows one installed with version 0.9.0 or later.
  The part that carries terminals, the hub, isn't on a Windows home computer installed before that. Refer to
  [Install Switchboard](install.md#a-windows-server).

### Open a chat's terminal

Select a chat on the board, then choose where its terminal opens.

| What you do | Where the terminal opens |
|---|---|
| Press ++enter++, or double-click the card | A tab |
| Press ++shift+enter++ | A window of its own |
| Press ++alt+enter++ (++option+enter++ on a Mac) | A floating sheet over the board |
| Press ++cmd+o++, then type a few letters of a chat's name | A tab, for any chat |

A chat that was stopped is resumed first, in the background on its computer, and then opens. Typing `exit`,
`/exit`, `quit` or `/quit` on an empty line leaves the chat and closes the tab. The chat keeps running.

### Open your own shell

Press ++cmd+t++ to open your default shell on the computer you're at. The arrow beside **+** at the end of the tab
row lists every shell Switchboard found:

- **Mac and Linux**: your login shell first, then the others listed in `/etc/shells`, such as zsh, bash and fish.
- **Windows**: PowerShell 7 and Git Bash if they're installed, Windows PowerShell, Command Prompt, and one entry
  for each WSL distribution.

A shell's tab says **This Mac** or **This PC** where a chat's tab names its computer. When the shell exits
cleanly, its tab closes.

### Arrange tabs

Right-click a tab to open its menu.

| Menu item | What it does |
|---|---|
| **Star tab** | Keeps the tab first in the row. **Unstar tab** undoes it |
| **Rename chat** | Changes the chat's name everywhere |
| **Add to new group**, **Add to group** | Puts the tab in a group |
| **Move to a new window** | Gives the terminal a window of its own |
| **Show as a floating sheet** | Shows the terminal over the board |
| **Close tab** | Closes the tab. The item says in brackets what happens to the chat or the shell |

You can also drag a tab to reorder it, or drag it down out of the row to give it its own window. ++cmd+1++ shows
the board, ++cmd+2++ to ++cmd+8++ show the tabs in the row's order, and ++cmd+9++ shows the last one.

Tabs narrow as the row fills, and then the row scrolls sideways. The **All tabs** button just before **+** lists
every open tab, starred ones first, then each group under its name. Click a row to go to that tab, or its cross
to close it.

### Group tabs

1. Right-click a tab and select **Add to new group**. The group takes the next unused color: Blue, Red, Yellow,
   Green, Pink, Purple, Cyan or Grey.
2. Type the group's name on its chip and press ++enter++.
3. Click the chip to fold the group, and click it again to unfold it.

Right-click the chip to rename the group, pick another color, move it left or right, **Ungroup** it or **Close
group**. Groups are the same on every computer you use the app on. Which tabs are open, and which groups are
folded, stay on each computer.

### Rename a chat

Double-click a tab's title, type the new name, and press ++enter++. ++esc++ puts the old name back. The name
changes everywhere the chat shows. Refer to [Chats and what they cost](chats.md).

### Copy and paste

- **Copy**: select text with the mouse and it's copied when you let go. ++cmd+c++ also copies on a Mac.
  ++ctrl+c++ copies while text is selected and interrupts the program when nothing is selected.
- **Paste**: press ++cmd+v++ on a Mac, or ++ctrl+v++ on Windows and Linux.
- **Right-click**: copies the selection, or pastes when nothing is selected.
- **Images**: an image on the clipboard is sent to the chat's computer, and its path there is pasted into the
  chat.

A program that uses the mouse itself receives your clicks and drags. Hold ++shift++ to select text and to
right-click as usual.

### Drag files in

Drop a file on a chat's terminal to send it to the chat's computer. Switchboard copies the file there and pastes
its path into the chat. A file can be up to 20 MB, and folders can't be sent. **Attach** in the terminal does the
same through a file picker.

On your own shell, a dropped file isn't copied anywhere. Its path is typed at the cursor.

## Change how terminals look {#look}

Open **Settings** and select **Terminal**.

| Setting | Choices | Default |
|---|---|---|
| Color scheme | Converge Dark, Converge Light, Tokyo Night, Tokyo Night Day, Nord, Dracula, Gruvbox Dark, Gruvbox Light, Solarized Dark, Solarized Light, GitHub Light, Catppuccin Latte, or **Match the app** | Converge Dark |
| **Default terminal** | Any shell found on this computer | The first shell in the list |
| **Text size** | 9 to 24 | 13 |
| **Cursor** | **Block**, **Bar**, **Underline** | **Block** |
| **Blink** | On or off | On |

Hover a color scheme to preview it in every open terminal. ++cmd+plus++ and ++cmd+minus++ change the text size
from inside a terminal, and ++cmd+0++ returns to the default.

The color scheme, cursor and blink are the same on every computer you use the app on. The default terminal and the
text size are kept on each computer. This is because shells and screens differ between computers.

Claude Code draws for a dark terminal. If you pick a light scheme, run `/theme` in Claude Code and pick a light
theme there too.

## Change what closing a tab does

Closing a chat's tab normally leaves the chat running, and **Stop** on its card ends it. To have a chat stop when
its tab closes, open **Settings**, select **Terminal**, and turn on **Stop a chat when its tab closes**. The
setting is off by default and applies to this computer only.

With the setting on, a chat stops with its tab once its reply is done. A chat set to **Keep running** is never
stopped this way, and **Close group** asks first when it would stop chats.

Closing your own shell always ends it. On a Mac and on Linux, Switchboard asks first when a program is still
running in the shell, and asks again if you quit the app. On Windows it can't tell, so it doesn't ask.

## Limitations

- A chat's terminal needs a Mac or Linux home computer today. With a Windows home computer you still get the
  board and your own shells.
- Only background chats open in the app. A chat that is open in a terminal on its own computer has to be typed
  into there.
- A file you drag in can be up to 20 MB.
- Chat names take English letters, digits, spaces and a few punctuation marks.
- Your own shells aren't saved with the tabs, so they don't reopen the next time you start the app.

## Troubleshooting

### The terminal says Reconnecting

The connection to the chat dropped. The chat itself is still running on its computer.

1. Wait. The terminal tries again by itself for about five minutes, and keeps what you could see and scroll back
   to.
2. To try at once, select **Reconnect now**, or press ++enter++ when you've typed nothing.

Keys you type while it reconnects are sent when it's back, if that happens within five seconds. After that they're
dropped and the bar says so. This is because an ++enter++ typed at a screen that has since changed shouldn't land
blind. A paste isn't kept: paste again once the terminal is back.

### The screen looks wrong

Press ++cmd+shift+r++, or select **Redraw the terminal** in the menu. Switchboard asks for a fresh copy of the
screen and draws it on a clean grid.

### "It's open in a terminal on" another computer

The chat is running in a terminal window on its own computer, not in the background. Type in it there, or select
**Open in Claude** when the chat has a page on claude.ai.
