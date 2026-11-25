# Install the Switchboard server on this Windows PC, for the person who runs it. No admin rights needed, except
# for the firewall rule it prints at the end.
#
#   powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1      (from a copy of the repository)
#
# The hub (terminals, and linking machines with a code) and switchboard-node come ready-made: native\dist\ in the
# copy this runs from carries switchboard-hub.exe and switchboard-node.exe when it was packed with them. Nothing is
# built here. Without a hub the server reaches machines over ssh only.
#
# What it puts in place:
#   %LOCALAPPDATA%\Switchboard\app\         the server (server\, agent\sb.py, install\setup.py)
#   %LOCALAPPDATA%\Switchboard\python\      Switchboard's own Python, when the kit carries one (python\ in the kit);
#                                           this PC's own Python 3.9 or newer is used only when it does not
#   %LOCALAPPDATA%\Switchboard\switchboard-hub.exe   the hub: terminals and linked machines
#   %LOCALAPPDATA%\Switchboard\downloads\   what other machines download when you add them (sb.py, install-agent.*,
#                                           switchboard-node for each kind of computer)
#   %LOCALAPPDATA%\Switchboard\switchboard.ps1   start, stop, status, logs, setup, link, keys
# and three logon tasks with no window: SwitchboardServer (:8130), SwitchboardHub (:8132) and SwitchboardDownloads
# (:8131).
#
# The first computer links itself: after the server has started, when the hub is here, this PC has no link yet and
# the server has no computers, it takes a code from its own hub and installs switchboard-node, so its own chats show
# on the board (switchboard.ps1 link --first; -NoLink skips it).
#
#   -HubBinary PATH   use this switchboard-hub.exe instead of the one in native\dist
#   -NodeDir DIR      take the switchboard-node builds for the downloads from here first, then from native\dist
#   -NoHub            skip the hub (no terminals, no linked machines; machines are reached over ssh)
#   -NoLink           don't link this PC to its own server
#   -Keys             turn keys on at the end (switchboard.ps1 keys on) and print a code for the next computer
#   -NoStart          install only
#   -Standby          install the parts a home computer needs and start nothing, now or at sign-in, so that this
#                     PC can take over as the home computer (switchboard-node home move, home restore). The same
#                     as -NoStart -NoTasks -NoLink -NoSetup.
#   -NoSetup          don't run first-run setup at the end
#   -NoTasks          no logon tasks: nothing starts when you sign in, and switchboard.ps1 start runs every part
#                     itself, switchboard-node too. With SWITCHBOARD_BASE set to another folder and the
#                     SWITCHBOARD_ ports set, this is a second server beside the real one (they are kept in
#                     env.txt there); the helper for Claude Code and the settings still go under USERPROFILE.
#   -Uninstall        stop and remove everything this put in place (settings and history are kept)
param([switch]$NoStart, [switch]$NoSetup, [switch]$Uninstall, [string]$HubBinary = '', [string]$NodeDir = '',
      [switch]$NoHub, [switch]$NoLink, [switch]$Keys, [switch]$NoTasks, [switch]$Standby)
if ($Standby) { $NoStart = $true; $NoTasks = $true; $NoLink = $true; $NoSetup = $true }
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Say([string]$m) { Write-Host $m }
function Step([string]$m) { Write-Host ''; Write-Host "== $m" }
function Die([string]$m) { Write-Host "install-server: $m" -ForegroundColor Red; exit 1 }

$Here = $PSScriptRoot
$Repo = (Resolve-Path (Join-Path $Here '..\..')).Path
$Base = if ($env:SWITCHBOARD_BASE) { $env:SWITCHBOARD_BASE } else { Join-Path $env:LOCALAPPDATA 'Switchboard' }
$App = Join-Path $Base 'app'
$Dl = Join-Path $Base 'downloads'
$Logs = Join-Path $Base 'logs'
$Ctl = Join-Path $Base 'switchboard.ps1'
$HubExe = Join-Path $Base 'switchboard-hub.exe'
$Tasks = @('SwitchboardServer', 'SwitchboardHub', 'SwitchboardDownloads')
$Port = if ($env:SWITCHBOARD_PORT) { $env:SWITCHBOARD_PORT } else { '8130' }
$DlPort = if ($env:SWITCHBOARD_DOWNLOADS_PORT) { $env:SWITCHBOARD_DOWNLOADS_PORT } else { '8131' }
$HubPort = if ($env:SWITCHBOARD_HUB_PORT) { $env:SWITCHBOARD_HUB_PORT } else { '8132' }
# Runs switchboard.ps1 and returns what it printed, a line each; $script:Rc is how it ended.
function Ctl-Out([string[]]$a) {
    $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $out = @(& powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl @a 2>&1 | ForEach-Object { "$_" }); $script:Rc = $LASTEXITCODE } catch { $out = @("$_"); $script:Rc = 1 }
    $ErrorActionPreference = $old
    return $out
}

if ($Uninstall) {
    if (Test-Path $Ctl) { & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl stop | Out-Null }
    if (-not (Test-Path (Join-Path $Base 'by-hand.txt'))) { foreach ($t in $Tasks) { schtasks /Delete /TN $t /F 2>$null | Out-Null } }
    # A switchboard-node on this PC lives in the folder the server is in, but this installer did not put it there:
    # it is what links a computer to a server, and that server may be another one. It goes with the server only
    # when node.json says its hub is this PC (unsure counts as no). Otherwise its files are the one thing left in
    # the folder, so the PC stays linked.
    $NodeExe = Join-Path $Base 'switchboard-node.exe'
    $NodeNote = @()
    $KeepNode = $false
    if (Test-Path $NodeExe) {
        $Here = $false
        try {
            $cfg = if ($env:SWITCHBOARD_NODE_CONFIG) { $env:SWITCHBOARD_NODE_CONFIG } else { Join-Path $HOME '.config\switchboard\node.json' }
            $hub = [string](Get-Content -Raw $cfg | ConvertFrom-Json).hub
            $h = ($hub -replace '^[a-zA-Z]+://', '') -replace '/.*$', ''
            if ($h -match '^\[([^\]]+)\]') { $h = $Matches[1] } elseif (($h -split ':').Count -eq 2) { $h = ($h -split ':')[0] }
            $h = $h.ToLower()
            $mine = @('localhost', '::1', $env:COMPUTERNAME, [System.Net.Dns]::GetHostName())
            $mine += [System.Net.NetworkInformation.NetworkInterface]::GetAllNetworkInterfaces() |
                ForEach-Object { $_.GetIPProperties().UnicastAddresses } | ForEach-Object { ($_.Address.ToString() -split '%')[0] }
            $Here = $h -and (($h -like '127.*') -or (($mine | Where-Object { $_ } | ForEach-Object { $_.ToLower() }) -contains $h))
        } catch { $Here = $false }
        if ($Here) {
            $out = @()
            try { $out = @(& $NodeExe uninstall 2>&1 | ForEach-Object { "$_" }) } catch { $out += "$_" }
            $NodeNote += 'This PC was linked to its own server with switchboard-node, which took itself out too:'
            $NodeNote += $out | ForEach-Object { "  $_" }
        } else {
            $KeepNode = $true
            $NodeNote += 'This PC is still linked to a Switchboard server as one of its computers: switchboard-node was left'
            $NodeNote += "as it is, in $Base. To remove that too, run: `"$NodeExe`" uninstall"
        }
    }
    # Switchboard's own Python stays while the helper for Claude Code is still here and runs with it.
    $KeepPy = (Test-Path (Join-Path $HOME '.claude\switchboard\sb.py')) -and (Test-Path (Join-Path $Base 'python'))
    if ($KeepNode -or $KeepPy) {
        Get-ChildItem -Force $Base | Where-Object { -not ($KeepNode -and $_.Name -like 'switchboard-node*') -and -not ($KeepPy -and $_.Name -eq 'python') } |
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    } else {
        Remove-Item -Recurse -Force $Base -ErrorAction SilentlyContinue
    }
    Say "Switchboard is removed from this PC. Your settings ($HOME\.config\switchboard) and history ($HOME\.local\share\switchboard) are still there;"
    Say 'delete those two folders too if you don''t want them.'
    foreach ($l in $NodeNote) { Say $l }
    exit 0
}

if (-not (Test-Path (Join-Path $Repo 'server\server.py')) -or -not (Test-Path (Join-Path $Repo 'agent\sb.py'))) {
    Die 'run this from a full copy of the Switchboard repository (no server\ or agent\ next to it).'
}

# ---------------------------------------------------------------- what this PC has
Step 'Checking this PC'
# Switchboard's own Python, in %LOCALAPPDATA%\Switchboard\python. An archive is unpacked beside it, tried, and moved
# into place; one that is here already and is the same build (its .pin) is kept as it is. When the folder cannot be
# replaced because a program still runs from it, the one that is here stays.
$OwnPy = Join-Path $Base 'python'
function PyRuns([string]$exe) {
    try { & $exe -c 'import sys, ssl, hashlib; sys.exit(0 if sys.version_info >= (3, 9) else 1)' 2>$null | Out-Null; return ($LASTEXITCODE -eq 0) } catch { return $false }
}
function OwnPython([string]$archive, [string]$pin) {
    $exe = Join-Path $OwnPy 'python.exe'
    # (one an earlier run could not delete, because a program still ran from it)
    if (Test-Path $OwnPy) { Remove-Item -Recurse -Force "$OwnPy.old" -ErrorAction SilentlyContinue }
    $has = (Test-Path $exe) -and (PyRuns $exe)
    $same = $has -and ((Get-Content (Join-Path $OwnPy '.pin') -ErrorAction SilentlyContinue | Select-Object -First 1) -eq $pin)
    if ($same) { return $exe }
    if ((Get-FileHash -Algorithm SHA256 $archive).Hash.ToLower() -ne $pin.ToLower()) { Die 'Python for this PC did not match its checksum; nothing was changed.' }
    $new = "$OwnPy.new"
    Remove-Item -Recurse -Force $new -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $new | Out-Null
    & tar.exe -xzf $archive -C $new --strip-components 1 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0 -or -not (PyRuns (Join-Path $new 'python.exe'))) {
        Remove-Item -Recurse -Force $new -ErrorAction SilentlyContinue
        if ($has) { return $exe }
        Say "(Switchboard's own Python does not run on this PC, so this PC's own is used.)"
        return $null
    }
    Set-Content -Path (Join-Path $new '.pin') -Value $pin -Encoding ascii
    try {
        if (Test-Path $OwnPy) { Remove-Item -Recurse -Force "$OwnPy.old" -ErrorAction SilentlyContinue; Move-Item $OwnPy "$OwnPy.old" }
        Move-Item $new $OwnPy
        Remove-Item -Recurse -Force "$OwnPy.old" -ErrorAction SilentlyContinue
    } catch {
        Remove-Item -Recurse -Force $new -ErrorAction SilentlyContinue
        if (-not (Test-Path $OwnPy) -and (Test-Path "$OwnPy.old")) { Move-Item "$OwnPy.old" $OwnPy }
        if ($has) { Say "(Switchboard's own Python is in use, so the one that is here stays until the next update.)"; return $exe }
        return $null
    }
    return $exe
}
function PinOf([string]$sums, [string]$name) {
    foreach ($l in ($sums -split "`n")) {
        $p = $l.Trim() -split '\s+', 2
        if ($p.Count -eq 2 -and $p[1].TrimStart('*') -eq $name) { return $p[0] }
    }
    return $null
}
$Py = $null
$KitPy = Join-Path $Repo 'python\python-windows-x86_64.tar.gz'
if ((Test-Path $KitPy) -and [Environment]::Is64BitOperatingSystem) {
    $pin = PinOf (Get-Content -Raw (Join-Path $Repo 'python\SHA256SUMS') -ErrorAction SilentlyContinue) 'python-windows-x86_64.tar.gz'
    if ($pin) {
        # A server that is running holds its Python's files, so it is stopped before a different build goes in; it
        # is started again below with everything else. (The helper's launcher may hold them too: see OwnPython.)
        $cur = Get-Content (Join-Path $OwnPy '.pin') -ErrorAction SilentlyContinue | Select-Object -First 1
        if ((Test-Path $OwnPy) -and $cur -ne $pin -and (Test-Path $Ctl)) {
            & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl stop | Out-Null
            Start-Sleep 1
        }
        $Py = OwnPython $KitPy $pin
    }
} elseif ((Test-Path (Join-Path $OwnPy 'python.exe')) -and (PyRuns (Join-Path $OwnPy 'python.exe'))) { $Py = Join-Path $OwnPy 'python.exe' }
foreach ($c in @('python', 'py')) {
    if ($Py) { break }
    $cmd = Get-Command $c -ErrorAction SilentlyContinue
    if (-not $cmd) { continue }
    $a = @(); if ($c -eq 'py') { $a = @('-3') }
    # (no double quotes in what Python is given: Windows PowerShell 5 drops them on the way to a program)
    try { $v = & $cmd.Source @a -c 'import sys; print(sys.version_info[0] * 100 + sys.version_info[1])' 2>$null } catch { $v = $null }
    if ("$v" -match '^\d+$' -and [int]"$v" -ge 309) { $Py = (& $cmd.Source @a -c 'import sys; print(sys.executable)').Trim(); break }
}
if (-not $Py) {
    Say 'Python 3 is not installed. Switchboard''s server is a Python program (3.9 or newer, nothing else to add).'
    Say 'Install it with:  winget install -e --id Python.Python.3.12'
    Say 'then open a new PowerShell and run this again.'
    exit 1
}
Say "Python at $Py"
if (Get-Command ssh -ErrorAction SilentlyContinue) { Say 'ssh is here (machines are added over ssh)' }
else { Die 'the OpenSSH client is missing. Turn it on in Settings, System, Optional features, OpenSSH Client, then run this again.' }
$Tz = try { [TimeZoneInfo]::Local.Id } catch { 'UTC' }
# Windows names its zones its own way; Python wants IANA names. .NET 6+ converts, Windows PowerShell 5 does not.
$Iana = $null
try { $tmp = $null; if ([TimeZoneInfo]::TryConvertWindowsIdToIanaId($Tz, [ref]$tmp)) { $Iana = $tmp } } catch { }
if (-not $Iana) { $Iana = 'UTC'; Say "Time zone: $Tz has no IANA name here; using UTC for the weekly usage days." } else { Say "Time zone $Iana" }
# Where the hub comes from: given (-HubBinary), or the ready-made one next to the server. None: no hub.
$HubSrc = $null
if (-not $NoHub) {
    if ($HubBinary) {
        if (-not (Test-Path $HubBinary)) { Die "there is no hub at $HubBinary." }
        $HubSrc = (Resolve-Path $HubBinary).Path
    } elseif (Test-Path (Join-Path $Repo 'native\dist\switchboard-hub.exe')) { $HubSrc = Join-Path $Repo 'native\dist\switchboard-hub.exe' }
}
if ($HubSrc) { Say 'The hub is here (terminals, and linking machines with a code)' }
elseif ($NoHub) { Say 'The hub is skipped (-NoHub): no terminals, and machines are added over ssh' }
elseif (Test-Path $HubExe) { Say 'This copy has no switchboard-hub.exe, so the hub that is installed stays as it is' }
else {
    Say 'This copy has no switchboard-hub.exe (native\dist), so there is no hub: no terminals, and machines are added'
    Say 'over ssh. To add it later, run this again with -HubBinary PATH.'
}

# ---------------------------------------------------------------- the server
Step 'Putting the server in place'
New-Item -ItemType Directory -Force $Base, $Logs | Out-Null
$New = Join-Path $Base 'app.new'
Remove-Item -Recurse -Force $New -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $New 'agent'), (Join-Path $New 'install') | Out-Null
Copy-Item -Recurse (Join-Path $Repo 'server') (Join-Path $New 'server')
Get-ChildItem -Recurse -Directory -Filter __pycache__ (Join-Path $New 'server') | Remove-Item -Recurse -Force
Copy-Item (Join-Path $Repo 'agent\sb.py') (Join-Path $New 'agent\sb.py')
Copy-Item (Join-Path $Here 'setup.py') (Join-Path $New 'install\setup.py')
# The server that is here is moved aside (app.old), not deleted, and stays until the new one has answered. Deleting
# it first left half a server when a file was still in use, and no server when the new one did not start.
$Old = Join-Path $Base 'app.old'
$HadOld = $false
function PutBack {
    if (-not $HadOld -or -not (Test-Path $Old)) { return $false }
    try {
        & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl stop | Out-Null
        Start-Sleep 1
        Remove-Item -Recurse -Force $App -ErrorAction SilentlyContinue
        Move-Item $Old $App
        if (Test-Path "$HubExe.old") { Move-Item "$HubExe.old" $HubExe -Force }
        & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl start | Out-Null
        return $true
    } catch { return $false }
}
# (a run cut short between the two moves below left no server, only app.old: that one goes back first)
if (-not (Test-Path $App) -and (Test-Path $Old)) { Move-Item $Old $App }
Remove-Item -Recurse -Force $Old -ErrorAction SilentlyContinue
if (Test-Path $Ctl) { & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl stop | Out-Null }
if (Test-Path $App) {
    # The server was stopped a moment ago and may hold its files a little longer: ask a few times.
    foreach ($i in 1..10) {
        try { Move-Item $App $Old; $HadOld = $true; break } catch { Start-Sleep 1 }
    }
    if (-not $HadOld) {
        if (Test-Path $Ctl) { & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl start | Out-Null }
        Die 'the server that is here could not be moved aside (is a program using its files?). Nothing was changed.'
    }
}
try { Move-Item $New $App } catch {
    if ($HadOld) { Move-Item $Old $App; & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl start | Out-Null }
    Die "could not put the new server in place ($($_.Exception.Message)). Nothing was changed."
}
Copy-Item (Join-Path $Here 'switchboard.ps1') $Ctl -Force
Set-Content -Path (Join-Path $Base 'python.txt') -Value $Py -Encoding ascii
Say "Server in $App"
# The hub goes in while everything is stopped. The one it replaces is kept (.old) until the new server has answered.
Remove-Item -Force "$HubExe.old" -ErrorAction SilentlyContinue
if ($HubSrc) {
    if ($HadOld -and (Test-Path $HubExe)) { Copy-Item $HubExe "$HubExe.old" -Force }
    foreach ($i in 1..10) {
        try { Copy-Item $HubSrc $HubExe -Force; break } catch { if ($i -eq 10) { Die "could not put the hub in place ($($_.Exception.Message))." }; Start-Sleep 1 }
    }
    Say "Hub in $HubExe"
} elseif ($NoHub) {
    Remove-Item -Force $HubExe, (Join-Path $Base 'SwitchboardHub.vbs') -ErrorAction SilentlyContinue
}
$HasHub = Test-Path $HubExe

Step 'Downloads for adding machines'
Remove-Item -Recurse -Force $Dl -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $Dl 'node') | Out-Null
Copy-Item (Join-Path $Repo 'agent\sb.py') (Join-Path $Dl 'node\sb.py')
# switchboard-node for each kind of computer: what a computer downloads when it joins with a code, and what links
# this PC to its own server.
$NodeNames = @('switchboard-node-mac', 'switchboard-node-linux', 'switchboard-node.exe')
foreach ($f in $NodeNames) {
    foreach ($d in @($NodeDir, (Join-Path $Repo 'native\dist'))) {
        if (-not $d) { continue }
        $src = Join-Path $d $f
        if ((Test-Path $src) -and (Get-Item $src).Length -gt 0) { Copy-Item $src (Join-Path $Dl "node\$f"); break }
    }
}
# Switchboard's own Python for the computers added to this server (install-agent.* downloads the one for its kind).
$KitPys = @(Get-ChildItem (Join-Path $Repo 'python') -Filter 'python-*.tar.gz' -File -ErrorAction SilentlyContinue)
if ($KitPys.Count) {
    New-Item -ItemType Directory -Force (Join-Path $Dl 'python') | Out-Null
    $KitPys | ForEach-Object { Copy-Item $_.FullName (Join-Path $Dl 'python') }
    $psums = Get-ChildItem (Join-Path $Dl 'python') -File | ForEach-Object { "$((Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLower())  $($_.Name)" }
    Set-Content -Path (Join-Path $Dl 'python\SHA256SUMS') -Value $psums -Encoding ascii
}
foreach ($f in 'install-agent.sh', 'install-agent.ps1') { Copy-Item (Join-Path $Here $f) (Join-Path $Dl $f) }
$sums = Get-ChildItem (Join-Path $Dl 'node') -File | ForEach-Object { "$((Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLower())  $($_.Name)" }
Set-Content -Path (Join-Path $Dl 'node\SHA256SUMS') -Value $sums -Encoding ascii
Say 'ready: sb.py and the install scripts'
foreach ($f in $NodeNames) {
    if (Test-Path (Join-Path $Dl "node\$f")) { Say "ready: $f" }
    elseif ($HasHub) { Say "missing: $f (Add a machine can't link that kind of machine with a code yet; see docs/install.md)" }
}

# ---------------------------------------------------------------- logon tasks
Step 'Running it'
$pyw = $Py  # python.exe, run hidden by wscript, so its output can go to a log
# Each part gets a small script that runs it with no window, and a logon task that runs the script. With -NoTasks
# only the script: switchboard.ps1 runs it (by-hand.txt tells it so), with the SWITCHBOARD_ settings this ran with
# (env.txt).
$ByHandNote = Join-Path $Base 'by-hand.txt'
if ($NoTasks) {
    Set-Content -Path $ByHandNote -Encoding ascii -Value 'install-server.ps1 -NoTasks: switchboard.ps1 starts and stops every part itself.'
    $kept = Get-ChildItem Env: | Where-Object { $_.Name -like 'SWITCHBOARD_*' -and $_.Name -ne 'SWITCHBOARD_BASE' } | ForEach-Object { "$($_.Name)=$($_.Value)" }
    Set-Content -Path (Join-Path $Base 'env.txt') -Encoding ascii -Value @($kept)
} else { Remove-Item -Force $ByHandNote -ErrorAction SilentlyContinue }
function Task([string]$name, [string]$cmdline, [string]$log, [string]$set = '') {
    $vbs = Join-Path $Base "$name.vbs"
    $inner = "cmd /c cd /d ""$App"" && set ""SWITCHBOARD_TZ=$Iana"" && $set$cmdline >> ""$log"" 2>&1"
    Set-Content -Path $vbs -Encoding ascii -Value ("CreateObject(""WScript.Shell"").Run """ + ($inner -replace '"', '""') + """, 0, False")
    if ($NoTasks) { return }
    schtasks /Create /F /SC ONLOGON /TN $name /TR "wscript.exe `"$vbs`"" /RL LIMITED | Out-Null
    if ($LASTEXITCODE -ne 0) { Die "could not create the logon task $name." }
}
Task 'SwitchboardServer' """$pyw"" -u ""$App\server\server.py""" (Join-Path $Logs 'server.log')
if ($HasHub) {
    # The hub listens on every interface, for the computers linked to it; with keys only it keeps to this PC by itself.
    $listen = if ($env:SWITCHBOARD_HUB_LISTEN) { $env:SWITCHBOARD_HUB_LISTEN } else { "0.0.0.0:$HubPort" }
    Task 'SwitchboardHub' """$HubExe""" (Join-Path $Logs 'hub.log') "set ""SWITCHBOARD_HUB_LISTEN=$listen"" && "
} elseif (-not $NoTasks) { schtasks /Delete /TN 'SwitchboardHub' /F 2>$null | Out-Null }
Task 'SwitchboardDownloads' """$pyw"" ""$App\server\downloads.py"" --port $DlPort --directory ""$Dl""" (Join-Path $Logs 'downloads.log')

if ($NoStart) {
    Remove-Item -Recurse -Force $Old -ErrorAction SilentlyContinue
    Remove-Item -Force "$HubExe.old" -ErrorAction SilentlyContinue
    if ($Standby) {
        Say 'Installed as a standby: nothing runs. This computer can take over as the home computer.'
        Say 'On the home computer, run: switchboard-node home move <this computer>'; exit 0
    }
    Say "Installed. Start it with: powershell -File `"$Ctl`" start"; exit 0
}
& powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl restart | Out-Null
$ok = $false
foreach ($i in 1..30) {
    try { Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/api/summary" -TimeoutSec 2 | Out-Null; $ok = $true; break } catch { Start-Sleep 1 }
}
& powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl status
if (-not $ok) {
    if (PutBack) { Die "the new server did not start, so the one that was here before was put back and started again. See: powershell -File `"$Ctl`" logs" }
    Die "the server did not start. See: powershell -File `"$Ctl`" logs"
}
Remove-Item -Recurse -Force $Old -ErrorAction SilentlyContinue
Remove-Item -Force "$HubExe.old" -ErrorAction SilentlyContinue

# ---------------------------------------------------------------- this PC, linked to its own server
if ($HasHub -and -not $NoLink) {
    Step 'Linking this PC'
    $said = @(Ctl-Out @('link', '--first'))
    if ($script:Rc -eq 0) { Say 'This computer is linked to its own server, so its chats show on the board.' }
    elseif ($script:Rc -eq 3) { Say "This server has other computers already, so this one was not linked. To link it too, run: powershell -File `"$Ctl`" link" }
    else {
        Say 'This computer could not be linked to its own server:'
        $said | ForEach-Object { Say "  $_" }
        Say "The server is installed and running. To link this computer, run: powershell -File `"$Ctl`" link"
    }
}

# ---------------------------------------------------------------- keys (-Keys)
# Turns keys on (switchboard.ps1 keys on: the mode in server.json, a restart, and a wait for the node here), then
# takes a code and a link for the next computer from the node. It never fails the install: the server runs either
# way, and what did not work is said with the line to run.
if ($Keys) {
    Step 'Keys'
    $said = @(Ctl-Out @('keys', 'on'))
    if ($script:Rc -ne 0) {
        Say 'Keys could not be turned on:'
        $said | ForEach-Object { Say "  $_" }
        Say "The server is installed and running. To turn keys on, run: powershell -File `"$Ctl`" keys on"
    } else {
        Say 'Keys are on: other computers can join this home computer with a code or a link.'
        $code = @(Ctl-Out @('code'))
        if ($script:Rc -ne 0) {
            Say 'A code for the next computer could not be made:'
            $code | ForEach-Object { Say "  $_" }
            Say "To get one, run: powershell -File `"$Ctl`" code"
        } else {
            Say "For the next computer (this works once, for a few minutes; 'switchboard.ps1 code' gives another):"
            $code | ForEach-Object { Say "  $_" }
            Say 'On that computer, run install-agent.sh (or install-agent.ps1 on Windows) with: --pair <the code or the link>'
        }
    }
}
Say ''
Say 'Other devices reach the server only if Windows Firewall lets them. In PowerShell opened with "Run as administrator":'
$fw = if ($HasHub) { "$Port,$DlPort,$HubPort" } else { "$Port,$DlPort" }
Say "  New-NetFirewallRule -DisplayName Switchboard -Direction Inbound -Protocol TCP -LocalPort $fw -Action Allow -Profile Private"
if ($Keys) { Say 'Computers that joined with a key need no rule: they connect to this PC with their key.' }

# ---------------------------------------------------------------- first-run setup
if (-not $NoSetup -and [Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
    Step 'First-run setup'
    & $Py (Join-Path $App 'install\setup.py')
} else {
    Say ''
    Say "Next: run  powershell -File `"$Ctl`" setup  to pick the server's address and add your machines."
}
