---
description: Switchboard on the home computer is not running after a restart or after you log out. The cause on Linux, on Windows and without systemd.
---

# Switchboard isn't running after a restart

<p class="sb-meta">Last validated: Nov 23, 2026</p>

After the home computer restarts, or after you log out of it, the app can't connect and `switchboard status`
says `not answering`.

## Cause

Switchboard runs as your own services, not as system services. The home computer is the one of your computers
that runs Switchboard itself, and what happens to those services depends on its system:

- **Linux**: the system stops your services when you log out, unless lingering is on for your account.
- **Linux without systemd**: in a container, or in WSL without systemd, there are no services. The installer
  starts plain background processes, and nothing starts them again.
- **Windows**: the two parts are logon tasks. They start when you sign in to Windows, not when the PC starts.
- **macOS**: the services are LaunchAgents. They start when you log in.

## Fix

=== "Linux"

    Turn lingering on once, so your services keep running while you are logged out:

    ```sh
    sudo loginctl enable-linger $USER
    ```

    Without systemd, start Switchboard by hand after each restart:

    ```sh
    switchboard start
    ```

    On a computer you added, start the helper the same way:

    ```sh
    ~/.local/bin/switchboard-node run &
    ```

=== "Windows"

    Sign in to Windows on the home computer. The two tasks start when you do.

=== "macOS"

    Log in on the home computer. The three services start when you do.

To check, run `switchboard status`.
