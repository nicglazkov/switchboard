# switchboard.ps1: start, stop and check the Switchboard server on this Windows PC, and link this PC to it.
# install-server.ps1 puts it in %LOCALAPPDATA%\Switchboard.
# Usage: switchboard.ps1 start|stop|restart|status|logs|setup|link|keys on|only|off|status|code|pair <code-or-link>|
#        devices|forget <name>|uninstall [--purge] [--yes]
# link links this PC to the server it runs, so its own chats show on the board. keys says how the other computers
# reach this one (server\keys.py); code, pair, devices and forget are switchboard-node's own, passed on.
# uninstall removes Switchboard from this PC: the helper's changes to Claude Code, switchboard-node, the server and
# its logon tasks, Switchboard's own Python, and the app when its uninstaller is here. Settings, history and keys
# stay unless --purge is given. It says what it removed and what it left.
param([string]$Verb = 'status', [Parameter(ValueFromRemainingArguments = $true)] $Rest)
$ErrorActionPreference = 'Stop'

# (SWITCHBOARD_BASE and the ports below are for this script's own tests, and for a server run on other ports beside
# the real one)
$Base = if ($env:SWITCHBOARD_BASE) { $env:SWITCHBOARD_BASE } else { Join-Path $env:LOCALAPPDATA 'Switchboard' }
# install-server.ps1 -NoTasks leaves this file: nothing starts when you sign in, and this command starts and stops
# every part itself, the node too.
$ByHand = Test-Path (Join-Path $Base 'by-hand.txt')
# What such a server runs with: NAME=VALUE lines, one SWITCHBOARD_ setting each, set before anything is started or
# asked. A setting that is already in the environment wins.
$EnvFile = Join-Path $Base 'env.txt'
if ($ByHand -and (Test-Path $EnvFile)) {
    foreach ($l in Get-Content $EnvFile) {
        if ($l -match '^\s*(SWITCHBOARD_[A-Z0-9_]+)=(.*)$' -and -not (Test-Path "Env:$($Matches[1])")) { Set-Item -Path "Env:$($Matches[1])" -Value $Matches[2].Trim() }
    }
}
$App = Join-Path $Base 'app'
$Logs = Join-Path $Base 'logs'
$Dl = Join-Path $Base 'downloads'
$Tasks = @('SwitchboardServer', 'SwitchboardHub', 'SwitchboardDownloads')
$Py = (Get-Content (Join-Path $Base 'python.txt') -ErrorAction SilentlyContinue | Select-Object -First 1)
# This person's folder, as the server finds it (USERPROFILE) and as switchboard-node and the hub do: a HOME that an
# ssh session, Git Bash or a test sets moves nothing. The link is looked for where the node keeps it, so a PC that
# is linked is never linked a second time.
$Me = if ($env:USERPROFILE) { $env:USERPROFILE } else { $HOME }
$NodeHome = $Me
$Cfg = Join-Path $Me '.config\switchboard'
$Port = if ($env:SWITCHBOARD_PORT) { $env:SWITCHBOARD_PORT } else { '8130' }
$DlPort = if ($env:SWITCHBOARD_DOWNLOADS_PORT) { $env:SWITCHBOARD_DOWNLOADS_PORT } else { '8131' }
$HubPort = if ($env:SWITCHBOARD_HUB_PORT) { $env:SWITCHBOARD_HUB_PORT } else { '8132' }
$HubUrl = if ($env:SWITCHBOARD_HUB_URL) { $env:SWITCHBOARD_HUB_URL } else { "http://127.0.0.1:$HubPort" }
$HubWait = if ($env:SWITCHBOARD_HUB_WAIT) { [int]$env:SWITCHBOARD_HUB_WAIT } else { 10 }
$NodeCfg = if ($env:SWITCHBOARD_NODE_CONFIG) { $env:SWITCHBOARD_NODE_CONFIG } else { Join-Path $NodeHome '.config\switchboard\node.json' }
$HubExe = Join-Path $Base 'switchboard-hub.exe'

# The processes, found by what they run (the logon tasks only start them, through a hidden script).
# The downloads were `python -m http.server 8131` before they were server\downloads.py: one started that way by an
# older install is still found, so an update stops it.
# The hub is found by where its program is. The node is this command's to stop only when it also started it
# (-NoTasks): one that switchboard-node installed runs from its own logon task and reads the mode by itself.
$NodeExe = Join-Path $Base 'switchboard-node.exe'
$KeysPy = Join-Path $App 'server\keys.py'
function Procs {
    Get-CimInstance Win32_Process -Filter "Name = 'python.exe' OR Name = 'pythonw.exe'" |
        Where-Object { $_.CommandLine -and ($_.CommandLine -like "*$App\server\server.py*" -or $_.CommandLine -like "*$App\server\downloads.py*" -or $_.CommandLine -like '*http.server 8131*') }
    $exes = @($HubExe); if ($ByHand) { $exes += $NodeExe }
    Get-CimInstance Win32_Process -Filter "Name = 'switchboard-hub.exe' OR Name = 'switchboard-node.exe'" |
        Where-Object { $_.ExecutablePath -and ($exes -contains $_.ExecutablePath) }
}

function Up([string]$url) {
    try { Invoke-WebRequest -UseBasicParsing -Uri $url -TimeoutSec 3 | Out-Null; 'running' } catch { 'not answering' }
}

# Each part has a small script that runs it with no window (install-server.ps1 writes them). The logon tasks run
# those; without tasks (-NoTasks) this command runs them itself.
function Start-Parts {
    if (-not $ByHand) {
        foreach ($t in $Tasks) { if (Test-Path (Join-Path $Base "$t.vbs")) { schtasks /Run /TN $t | Out-Null } }
        return
    }
    foreach ($t in ($Tasks + 'SwitchboardNode')) {
        $vbs = Join-Path $Base "$t.vbs"
        if (Test-Path $vbs) { Start-Process -FilePath 'wscript.exe' -ArgumentList "`"$vbs`"" | Out-Null }
    }
}
function Stop-Parts { Procs | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue } }

# Runs a program and returns what it printed, a line each; $script:Rc is how it ended. What it prints as an error
# is kept with the rest (and is not an error of this script).
function Run-Out([string]$exe, [string[]]$a) {
    $old = $ErrorActionPreference; $ErrorActionPreference = 'Continue'
    try { $out = @(& $exe @a 2>&1 | ForEach-Object { "$_" }); $script:Rc = $LASTEXITCODE } catch { $out = @("$_"); $script:Rc = 1 }
    $ErrorActionPreference = $old
    return $out
}

# off, both or only; off where this server knows nothing of keys
function Keys-Mode {
    if (-not $Py -or -not (Test-Path $KeysPy)) { return 'off' }
    try { $m = (& $Py $KeysPy mode 2>$null | Select-Object -First 1) } catch { $m = '' }
    if ($m -in 'both', 'only') { return $m }
    return 'off'
}

function Status {
    $keys = Keys-Mode
    Write-Host 'Switchboard on this PC'
    Write-Host "  server     http://127.0.0.1:$Port  $(Up "http://127.0.0.1:$Port/api/summary")"
    if (Test-Path $HubExe) { Write-Host "  hub        http://127.0.0.1:$HubPort  $(Up "http://127.0.0.1:$HubPort/healthz")" }
    else { Write-Host '  hub        not installed (terminals and linked machines are off)' }
    if ($keys -eq 'only') { Write-Host '  downloads  not served (keys only)' }
    else { Write-Host "  downloads  http://127.0.0.1:$DlPort  $(Up "http://127.0.0.1:$DlPort/node/SHA256SUMS")" }
    if ($keys -eq 'both') { Write-Host '  keys       on: computers connect with a key or by address' }
    if ($keys -eq 'only') { Write-Host '  keys       only: computers connect with a key, and the ports are closed to the network' }
}

# keys on|only|off|status. The mode is one word in server.json (server\keys.py writes it, and refuses "only" while
# that would cut a computer off); the server, the hub and the downloads read it when they start, so they are started
# again. A home computer that others join with a key needs the hub beside the server and switchboard-node linked to
# it: without either, turning keys on is refused with what to run. Status and off always work.
function Keys {
    $what = if ($Rest.Count) { [string]$Rest[0] } else { 'status' }
    $more = @($Rest | Select-Object -Skip 1)
    $usage = 'usage: switchboard.ps1 keys on|only [--force]|off|status'
    if ($what -notin 'on', 'only', 'off', 'status') { Write-Host $usage; exit 2 }
    if ($more.Count -and -not ($what -eq 'only' -and $more.Count -eq 1 -and $more[0] -eq '--force')) { Write-Host $usage; exit 2 }
    if (-not $Py -or -not (Test-Path $KeysPy)) { Write-Host 'The server is not installed on this PC.'; exit 1 }
    if ($what -eq 'status') { & $Py $KeysPy status; exit $LASTEXITCODE }
    if ($what -ne 'off') {
        if (-not (Test-Path $HubExe)) {
            Write-Host 'Keys need the hub on this home computer, and it is not installed here.'
            Write-Host 'Run install-server.ps1 again from a copy of Switchboard that has switchboard-hub.exe, or with -HubBinary PATH.'
            exit 1
        }
        if (-not (Test-Path $NodeExe)) {
            Write-Host 'Keys need switchboard-node on this home computer, and it is not installed here.'
            Write-Host 'Link this computer first: switchboard.ps1 link'
            exit 1
        }
    }
    & $Py $KeysPy $what @more
    if ($LASTEXITCODE -eq 3) { exit 0 }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Stop-Parts; Start-Sleep 1; Start-Parts
    & $Py $KeysPy wait
    exit $LASTEXITCODE
}

# ---------------------------------------------------------------- link this PC to its own server
$AskPy = @'
import json, sys, urllib.request
what, where = sys.argv[1], sys.argv[2]
if what == 'fleet':  # how many computers fleet.json lists; 0 with no file, -1 when it cannot be read
    try:
        d = json.load(open(where, encoding='utf-8'))
    except FileNotFoundError:
        print(0)
        sys.exit(0)
    except Exception:
        print(-1)
        sys.exit(0)
    rows = d if isinstance(d, list) else d.get('machines') if isinstance(d, dict) else None
    print(len(rows) if isinstance(rows, list) else -1)
    sys.exit(0)
path, data = ('/internal/nodes', None) if what == 'nodes' else ('/internal/pair', b'{}')
req = urllib.request.Request(where + path, data=data, headers={'Content-Type': 'application/json'})
try:  # straight there, never through a proxy from the environment
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req, timeout=5) as r:
        v = json.loads(r.read() or b'{}')
    print(len(v['nodes']) if what == 'nodes' else v['code'])
except Exception:
    sys.exit(1)
'@
# nodes: how many computers the hub has linked. code: a new code. fleet: how many computers the server lists.
# Nothing when it cannot say.
function Ask([string]$what) {
    $where = if ($what -eq 'fleet') { Join-Path $Cfg 'fleet.json' } else { $HubUrl }
    $out = @(Run-Out $Py @('-c', $AskPy, $what, $where))
    if ($script:Rc -ne 0 -or -not $out.Count) { return $null }
    return ([string]$out[0]).Trim()
}

# Writes the small script that runs one part with no window, its output in a log.
function Runner([string]$name, [string]$cmdline, [string]$log) {
    $inner = "cmd /c cd /d ""$App"" && $cmdline >> ""$log"" 2>&1"
    Set-Content -Path (Join-Path $Base "$name.vbs") -Encoding ascii -Value ("CreateObject(""WScript.Shell"").Run """ + ($inner -replace '"', '""') + """, 0, False")
}

# Links this PC to the server it runs, so its own chats show on the board: the helper for Claude Code, a code from
# the hub (which gives one only to this PC), and switchboard-node. With --first (what install-server.ps1 asks after
# a start) it does so only when the server has no computers yet. Ends 0 when it linked, 3 when there was nothing to
# do, 1 with the reason when it could not. A PC with a node.json is never linked a second time.
function Link {
    $first = ($Rest.Count -gt 0 -and [string]$Rest[0] -eq '--first')
    $node = Join-Path $Dl 'node\switchboard-node.exe'
    $sb = Join-Path $NodeHome '.claude\switchboard\sb.py'
    if (-not (Test-Path $HubExe)) {
        if ($first) { exit 3 }
        Write-Host 'This server has no hub, so computers are added over ssh. Run: switchboard.ps1 setup'; exit 1
    }
    if ((Test-Path $NodeCfg) -and (Test-Path $NodeExe)) {
        if (-not $first) { Write-Host "This computer is linked already. To link it again, first run: `"$NodeExe`" uninstall" }
        exit 3
    }
    # A link that is here with no switchboard-node beside this server belongs to a node installed somewhere else on
    # this PC (this is a server run from another folder beside the real one): pairing would replace that link.
    if ($ByHand -and (Test-Path $NodeCfg)) {
        Write-Host "This computer is linked already ($NodeCfg), by a switchboard-node that is not this server's. It is left as it is."
        exit 1
    }
    if (-not $Py) { Write-Host 'Python 3 was not found.'; exit 1 }
    if (-not (Test-Path $NodeCfg)) {
        # (the hub was started a moment ago, so it gets a few seconds to answer)
        $n = $null
        foreach ($i in 1..$HubWait) { $n = Ask 'nodes'; if ($null -ne $n) { break }; Start-Sleep 1 }
        if ($null -eq $n) { Write-Host 'The hub on this computer did not answer.'; exit 1 }
        if ($first -and -not ($n -eq '0' -and (Ask 'fleet') -eq '0')) { exit 3 }
    } elseif ($first) { exit 3 }  # linked by an earlier run whose last step failed: only `link` finishes it
    if (-not (Test-Path $node) -or (Get-Item $node).Length -eq 0) { Write-Host 'This server has no switchboard-node for this kind of computer.'; exit 1 }
    if (-not (Test-Path $sb)) {
        New-Item -ItemType Directory -Force (Split-Path $sb) | Out-Null
        Copy-Item (Join-Path $App 'agent\sb.py') $sb -Force
        $out = @(Run-Out $Py @($sb, 'install'))
        if ($script:Rc -ne 0) { Remove-Item -Force $sb -ErrorAction SilentlyContinue; Write-Host "The helper for Claude Code could not be set up: $($out -join ' ')"; exit 1 }
    }
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ('sb-link-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force $tmp | Out-Null
    $new = Join-Path $tmp 'switchboard-node.exe'
    Copy-Item $node $new
    try {
        if (-not (Test-Path $NodeCfg)) {
            $code = Ask 'code'
            if (-not $code) { Write-Host 'The hub on this computer gave no code.'; exit 1 }
            $out = @(Run-Out $new @('pair', ($HubUrl -replace '^[a-zA-Z]+://', ''), $code))
            if ($script:Rc -ne 0) { $out | ForEach-Object { Write-Host $_ }; exit 1 }
        }
        if ($ByHand) {  # nothing starts when you sign in: a background process, like the server itself here
            Copy-Item $new $NodeExe -Force
            New-Item -ItemType Directory -Force $Logs | Out-Null
            Runner 'SwitchboardNode' """$NodeExe"" run" (Join-Path $Logs 'node.log')
            Start-Process -FilePath 'wscript.exe' -ArgumentList "`"$(Join-Path $Base 'SwitchboardNode.vbs')`"" | Out-Null
        } else {
            $out = @(Run-Out $new @('install'))
            if ($script:Rc -ne 0) {
                $out | ForEach-Object { Write-Host $_ }
                Write-Host 'The link is saved, but switchboard-node could not be set to run when you sign in.'; exit 1
            }
        }
    } finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
    Write-Host 'This computer is linked to its own server.'
    exit 0
}

# code, pair, devices and forget are switchboard-node's own: this passes them on, so there is one command to know.
function Node-Cmd {
    if (-not (Test-Path $NodeExe)) {
        Write-Host 'switchboard-node is not installed on this PC. On the home computer, run: switchboard.ps1 link'
        Write-Host 'To join another home computer instead: install-agent.ps1 -Pair <code or link>'
        exit 1
    }
    & $NodeExe $Verb @Rest
    exit $LASTEXITCODE
}

function Uninstall {
    $purge = @($Rest | Where-Object { $_ -in '--purge', '-Purge', '-purge' }).Count -gt 0
    $yes = @($Rest | Where-Object { $_ -in '--yes', '-y', '-Yes', '-yes' }).Count -gt 0
    $odd = @($Rest | Where-Object { $_ -notin '--purge', '-Purge', '-purge', '--yes', '-y', '-Yes', '-yes' })
    if ($odd.Count) { Write-Host 'usage: switchboard.ps1 uninstall [--purge] [--yes]'; exit 2 }
    if (-not $yes -and [Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
        $q = if ($purge) { 'Remove Switchboard from this PC, with its settings, history and keys? [y/N]' } else { 'Remove Switchboard from this PC? Settings and history are kept. [y/N]' }
        if ((Read-Host $q) -notmatch '^(y|yes)$') { Write-Host 'Nothing was removed.'; exit 1 }
    }
    $removed = @(); $left = @()
    $hadKeys = $false
    $HelperDir = Join-Path $HOME '.claude\switchboard'
    $Sb = Join-Path $HelperDir 'sb.py'
    $helperLeft = $false

    # 1. the helper: Claude Code's hooks, status line and tools go back to how they were
    if (Test-Path $Sb) {
        $ok = $false
        if ($Py) { try { & $Py $Sb uninstall 2>&1 | Out-Null; $ok = ($LASTEXITCODE -eq 0) } catch { $ok = $false } }
        if ($ok) {
            Remove-Item -Force $Sb -ErrorAction SilentlyContinue
            $removed += "the helper: its hooks, status line and tools are out of Claude Code ($HOME\.claude)"
        } else {
            $helperLeft = $true
            $left += "$Sb  the helper, which could not take itself out of Claude Code"
        }
    }

    # 2. switchboard-node, which links this PC to a server
    $Node = Join-Path $Base 'switchboard-node.exe'
    if (Test-Path $Node) {
        $out = @(); $ok = $false
        $hadKeys = (Test-Path (Join-Path $HOME '.config\switchboard\device.key')) -or (Test-Path (Join-Path $HOME '.local\share\switchboard\paired.json'))
        $nodeArgs = @('uninstall'); if ($purge) { $nodeArgs += '--purge' }  # (its key and its paired devices stay unless --purge)
        try { $out = @(& $Node @nodeArgs 2>&1 | ForEach-Object { "$_" }); $ok = ($LASTEXITCODE -eq 0) } catch { $out += "$_" }
        if ($ok) { $removed += 'switchboard-node, which linked this PC to a server'; $removed += $out | ForEach-Object { "  $_" } }
        else { $left += "$Node  it could not remove itself. Run: `"$Node`" uninstall"; $left += $out | ForEach-Object { "  $_" } }
    }

    # 3. the server, its downloads and their logon tasks; then Switchboard's own Python, which nothing uses once
    #    the helper and the server are gone (a program still running from it is stopped first)
    Stop-Parts
    foreach ($t in $Tasks) { schtasks /Delete /TN $t /F 2>$null | Out-Null }
    if (Test-Path $App) { $removed += 'the server, its downloads and the logon tasks that started them' }
    $OwnPy = Join-Path $Base 'python'
    $keep = @()
    if (Test-Path $Node) { $keep += 'switchboard-node*' }
    if (Test-Path $OwnPy) {
        if ($helperLeft) { $keep += 'python'; $left += "$OwnPy  Switchboard's own Python, which the helper still uses" }
        else {
            Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and $_.ExecutablePath -like "$OwnPy\*" -and $_.ProcessId -ne $PID } |
                ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
            $removed += "Switchboard's own Python"
        }
    }
    Get-ChildItem -Force $Base -ErrorAction SilentlyContinue | Where-Object { $n = $_.Name; -not ($keep | Where-Object { $n -like $_ }) } |
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
    if (-not (Get-ChildItem -Force $Base -ErrorAction SilentlyContinue)) { Remove-Item -Force $Base -ErrorAction SilentlyContinue }
    $removed += 'the switchboard command (switchboard.ps1)'

    # 4. the app, through its own uninstaller when it is here
    $AppUn = Join-Path $env:LOCALAPPDATA 'Programs\Switchboard\Uninstall Switchboard.exe'
    if (Test-Path $AppUn) {
        $a = @('--uninstall', '--quiet'); if ($purge) { $a += '--remove-data' }
        try { $p = Start-Process -FilePath $AppUn -ArgumentList $a -Wait -PassThru; $ok = ($p.ExitCode -eq 0) } catch { $ok = $false }
        if ($ok) { $removed += 'the Switchboard app' } else { $left += "$(Split-Path $AppUn)  the app. Remove it in Settings, Apps." }
    }

    # 5. settings, history and keys
    foreach ($d in @(@((Join-Path $HOME '.config\switchboard'), 'settings and keys'), @((Join-Path $HOME '.local\share\switchboard'), 'history and logs'),
                     @($HelperDir, "the helper's own files"))) {
        if (-not (Test-Path $d[0])) { continue }
        if ($purge -and -not ($helperLeft -and $d[0] -eq $HelperDir)) {
            Remove-Item -Recurse -Force $d[0] -ErrorAction SilentlyContinue
            if (-not (Test-Path $d[0])) { $removed += "$($d[0])  $($d[1])"; continue }
        }
        $left += "$($d[0])  $($d[1])"
    }

    Write-Host 'Switchboard is removed from this PC.'
    Write-Host 'Removed:'
    $removed | ForEach-Object { Write-Host "  $_" }
    if ($left.Count) {
        Write-Host 'Left on this PC:'
        $left | ForEach-Object { Write-Host "  $_" }
        if (-not $purge) { Write-Host 'Settings, history and keys are kept in case you install it again. Delete those folders if you do not want them.' }
    } else { Write-Host 'Nothing of Switchboard is left on this PC.' }
    if ($purge -and $hadKeys -and -not (Test-Path (Join-Path $HOME '.config\switchboard\device.key'))) {
        Write-Host "This computer's key and its list of paired devices are removed: every device that was paired with this computer will need to pair again."
    }
    Write-Host 'Your other computers each have their own copy. Run this on them too.'
}

switch ($Verb) {
    'start' { Start-Parts; Start-Sleep 3; Status }
    'stop' { Stop-Parts; Write-Host 'Stopped.' }
    'restart' { Stop-Parts; Start-Sleep 1; Start-Parts; Start-Sleep 3; Status }
    'status' { Status }
    'logs' { foreach ($p in 'server', 'hub', 'downloads', 'node') { $f = Join-Path $Logs "$p.log"; if (Test-Path $f) { "== $p"; Get-Content $f -Tail 30 } } }
    'setup' { & $Py (Join-Path $App 'install\setup.py') @Rest }
    'link' { Link }
    'keys' { Keys }
    { $_ -in 'code', 'pair', 'devices', 'forget', 'home' } { Node-Cmd }
    'uninstall' { Uninstall }
    default { Write-Host 'usage: switchboard.ps1 start|stop|restart|status|logs|setup|link|keys on|only|off|status|code|pair <code-or-link>|devices|forget <name>|home status|move|backup|restore|uninstall [--purge]'; exit 2 }
}
