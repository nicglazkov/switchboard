---
description: Go from nothing to answering a Claude Code chat from another computer. Install Switchboard on one computer, add the others with one line each, and open the app.
---

# Quickstart

<p class="sb-meta">Last validated: Nov 23, 2026</p>

This quickstart takes you from nothing to answering a chat from another computer. You install Switchboard on one
of your computers, add the computers that run Claude Code, and open the app. It takes about ten minutes.

## How it works

One of your computers, known as the home computer, keeps the board. Every computer that runs Claude Code gets a
small helper that reports its chats to the home computer. The app shows the board and sends back what you type.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/quickstart-flow.svg"
<figcaption>Figure 1. Three steps set Switchboard up, on the top row. Three more carry a question to you and your
answer back, on the bottom row.</figcaption>
</figure>

1. You install Switchboard on the home computer.
2. You paste one line on each computer that runs Claude Code. Its helper links to the home computer.
3. You open the app on the computer in front of you. The app finds the home computer.
4. A chat on another computer asks a question. Its helper reports it, and the board moves the chat to
   <span class="sb-pill need">Needs you</span>.
5. You answer on the board.
6. The home computer passes your answer to the chat, on the computer where it runs.

## Get started

To set up Switchboard and answer your first chat:

1. Install Switchboard on your home computer.
2. Pick the home computer's address.
3. Add your computers.
4. Get the app.
5. Start a chat and answer it.

### Prerequisites

Before you can set up Switchboard, you must:

- Pick a home computer: a Mac or a Linux computer that stays on, such as a desktop that never sleeps. With one
  computer only, that computer is the home computer too.
- Have Python 3.9 or newer on the home computer and on each computer you add.
- Have Claude Code installed and signed in on each computer where you want chats.
- Have all of them on the same home network, or on Tailscale, a private network between your own devices.

A Mac has Python once the command line tools are installed, which `xcode-select --install` does. Most Linux
systems come with it. On Windows, run `winget install -e --id Python.Python.3.12`.

### Install Switchboard on your home computer

=== "macOS"

    Open Terminal on the home computer and run:

    ```sh
    curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash -s -- --kit https://github.com/nicglazkov/switchboard/releases/latest/download/switchboard-server-kit.tar.gz
    ```

=== "Linux"

    Open a terminal on the home computer and run:

    ```sh
    curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash -s -- --kit https://github.com/nicglazkov/switchboard/releases/latest/download/switchboard-server-kit.tar.gz
    ```

    Linux stops your services when you log out. To keep Switchboard running, run this once:

    ```sh
    sudo loginctl enable-linger $USER
    ```

=== "Windows"

    A Windows PC can be the home computer, but with less today: it has no chat terminals, and it adds computers
    over ssh only. For this quickstart, use a Mac or a Linux computer as the home computer and add your Windows PC in
    a later step. To use Windows anyway, refer to [Install Switchboard](install.md#a-windows-server).

The installer checks the computer, installs Switchboard for your account only, and starts it. It needs no
administrator password. When it is done, it starts first-run setup in the same window.

### Pick the home computer's address

Setup's first question is the address your other devices use to reach the home computer. It lists the addresses
it found: the computer's address on your network, its Tailscale name when Tailscale is running, and its own
name.

Pick the address on your network if every device stays at home. Pick the Tailscale name if you also want to
reach the board away from home.

### Add your computers

Setup's second question is **your machines**. A machine is what setup calls a computer.

1. Choose **Add a machine with a code**. Setup prints a code and one line for each system, then waits.
2. On the computer you are adding, paste the line for its system.

    === "macOS"

        In Terminal:

        ```sh
        curl -fsSL http://192.0.2.10:8131/install-agent.sh | bash -s -- --server 192.0.2.10 --code ABC234
        ```

    === "Linux"

        In a terminal:

        ```sh
        curl -fsSL http://192.0.2.10:8131/install-agent.sh | bash -s -- --server 192.0.2.10 --code ABC234
        ```

    === "Windows"

        In PowerShell:

        ```powershell
        & ([scriptblock]::Create((iwr -UseBasicParsing http://192.0.2.10:8131/install-agent.ps1).Content)) -Server 192.0.2.10 -Code ABC234
        ```

    Copy the line from setup, not from this page. Setup fills in your home computer's address in place of
    `192.0.2.10` and a real code in place of `ABC234`.

3. Wait for setup to say **Linked**, with the computer's name.
4. Choose **Add a machine with a code** again for the next computer, or **Done**.

A code works once, for 10 minutes. This is because the code is all a computer needs to join.

If you use Claude Code on the home computer itself, add it the same way. Open a second terminal window there and
paste the line.

Setup's last question is each computer's Claude folder, where new chats start. Press ++enter++ to take the home
folder, or type a number to open a folder first.

### Get the app

Download the app for the computer in front of you from the
[latest release](https://github.com/nicglazkov/switchboard/releases/latest).

=== "macOS"

    Open `Switchboard-<version>.dmg` and double-click **Install Switchboard**.

=== "Linux"

    Unpack `Switchboard-linux-x86_64.tar.gz` and run `Switchboard` from the folder.

=== "Windows"

    Run `Switchboard-Setup-<version>.exe`. If Windows says **Windows protected your PC**, select **More info**,
    then **Run anyway**.

On the same network as the home computer, the app finds it and connects by itself. If it doesn't, choose **Type
an address** and enter the address you picked in setup, such as `http://192.0.2.10:8130`. Replace `192.0.2.10`
with your home computer's address.

For more information, refer to [The apps](apps.md).

### Start a chat and answer it

1. Press ++n++ on the board to open **New chat**.
2. Under **Machine**, pick a computer other than the one in front of you.
3. Write a first message that makes Claude ask something, such as "Ask me which language to use, then write
   hello world in it."
4. Select **Start**.

The chat shows on the board in <span class="sb-pill run">Working</span>. When Claude asks its question, the chat
moves to <span class="sb-pill need">Needs you</span>. Select the button on its card to answer. Your answer goes
to the chat on the other computer, and the chat moves back to **Working**.

For more information, refer to [Answer from the board](features/answer-from-the-board.md).

## Next steps

- **Learn the board**: refer to [The board](board.md).
- **Know what a reply costs**: refer to [Chats and what they cost](chats.md).
- **Open a chat's terminal**: refer to [Terminals](terminals.md).
- **Reach the board away from home**: refer to [Install Switchboard](install.md#away-from-home).
- **Fix a problem**: refer to [Troubleshooting](troubleshooting.md).
