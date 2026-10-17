# Install the Switchboard server on this Windows PC, for the person who runs it. No admin rights needed, except
# for the firewall rule it prints at the end.
#
#   powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1      (from a copy of the repository)
#
# On Windows the server reaches machines over ssh only: the hub (terminals, and linking machines with a code) does
# not run on Windows yet. For everything, run the server on a Mac or a Linux machine (install-server.sh); every
# Windows PC can still be one of your machines (install-agent.ps1).
#
# What it puts in place:
#   %LOCALAPPDATA%\Switchboard\app\         the server (server\, agent\sb.py, install\setup.py)
#   %LOCALAPPDATA%\Switchboard\downloads\   what other machines download when you add them (sb.py, install-agent.*)
#   %LOCALAPPDATA%\Switchboard\switchboard.ps1   start, stop, status, logs, setup
# and two logon tasks with no window: SwitchboardServer (:8130) and SwitchboardDownloads (:8131).
#
#   -NoStart     install only
#   -NoSetup     don't run first-run setup at the end
#   -Uninstall   stop and remove everything this put in place (settings and history are kept)
param([switch]$NoStart, [switch]$NoSetup, [switch]$Uninstall)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Say([string]$m) { Write-Host $m }
function Step([string]$m) { Write-Host ''; Write-Host "== $m" }
function Die([string]$m) { Write-Host "install-server: $m" -ForegroundColor Red; exit 1 }

$Here = $PSScriptRoot
$Repo = (Resolve-Path (Join-Path $Here '..\..')).Path
$Base = Join-Path $env:LOCALAPPDATA 'Switchboard'
$App = Join-Path $Base 'app'
$Dl = Join-Path $Base 'downloads'
$Logs = Join-Path $Base 'logs'
$Ctl = Join-Path $Base 'switchboard.ps1'
$Tasks = @('SwitchboardServer', 'SwitchboardDownloads')

if ($Uninstall) {
    if (Test-Path $Ctl) { & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl stop | Out-Null }
    foreach ($t in $Tasks) { schtasks /Delete /TN $t /F 2>$null | Out-Null }
    Remove-Item -Recurse -Force $Base -ErrorAction SilentlyContinue
    Say "Switchboard is removed from this PC. Your settings ($HOME\.config\switchboard) and history ($HOME\.local\share\switchboard) are still there;"
    Say 'delete those two folders too if you don''t want them.'
    exit 0
}

if (-not (Test-Path (Join-Path $Repo 'server\server.py')) -or -not (Test-Path (Join-Path $Repo 'agent\sb.py'))) {
    Die 'run this from a full copy of the Switchboard repository (no server\ or agent\ next to it).'
}

# ---------------------------------------------------------------- what this PC has
Step 'Checking this PC'
$Py = $null
foreach ($c in @('python', 'py')) {
    $cmd = Get-Command $c -ErrorAction SilentlyContinue
    if (-not $cmd) { continue }
    $a = @(); if ($c -eq 'py') { $a = @('-3') }
    try { $v = & $cmd.Source @a -c 'import sys; print("%d.%d" % sys.version_info[:2])' 2>$null } catch { $v = $null }
    if ($v -and [version]$v -ge [version]'3.9') { $Py = (& $cmd.Source @a -c 'import sys; print(sys.executable)').Trim(); break }
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
if (Test-Path $Ctl) { & powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl stop | Out-Null }
Remove-Item -Recurse -Force $App -ErrorAction SilentlyContinue
Move-Item $New $App
Copy-Item (Join-Path $Here 'switchboard.ps1') $Ctl -Force
Set-Content -Path (Join-Path $Base 'python.txt') -Value $Py -Encoding ascii
Say "Server in $App"

Step 'Downloads for adding machines'
Remove-Item -Recurse -Force $Dl -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $Dl 'node') | Out-Null
Copy-Item (Join-Path $Repo 'agent\sb.py') (Join-Path $Dl 'node\sb.py')
foreach ($f in 'install-agent.sh', 'install-agent.ps1') { Copy-Item (Join-Path $Here $f) (Join-Path $Dl $f) }
$sums = Get-ChildItem (Join-Path $Dl 'node') -File | ForEach-Object { "$((Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLower())  $($_.Name)" }
Set-Content -Path (Join-Path $Dl 'node\SHA256SUMS') -Value $sums -Encoding ascii
Say 'ready: sb.py and the install scripts (machines join over ssh; linking with a code needs a Mac or Linux server)'

# ---------------------------------------------------------------- logon tasks
Step 'Running it'
$pyw = $Py  # python.exe, run hidden by wscript, so its output can go to a log
function Task([string]$name, [string]$cmdline, [string]$log) {
    $vbs = Join-Path $Base "$name.vbs"
    $inner = "cmd /c cd /d ""$App"" && set ""SWITCHBOARD_TZ=$Iana"" && $cmdline >> ""$log"" 2>&1"
    Set-Content -Path $vbs -Encoding ascii -Value ("CreateObject(""WScript.Shell"").Run """ + ($inner -replace '"', '""') + """, 0, False")
    schtasks /Create /F /SC ONLOGON /TN $name /TR "wscript.exe `"$vbs`"" /RL LIMITED | Out-Null
    if ($LASTEXITCODE -ne 0) { Die "could not create the logon task $name." }
}
Task 'SwitchboardServer' """$pyw"" -u ""$App\server\server.py""" (Join-Path $Logs 'server.log')
Task 'SwitchboardDownloads' """$pyw"" -m http.server 8131 --directory ""$Dl""" (Join-Path $Logs 'downloads.log')

if ($NoStart) { Say "Installed. Start it with: powershell -File `"$Ctl`" start"; exit 0 }
& powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl restart | Out-Null
$ok = $false
foreach ($i in 1..30) {
    try { Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:8130/api/summary' -TimeoutSec 2 | Out-Null; $ok = $true; break } catch { Start-Sleep 1 }
}
& powershell -NoProfile -ExecutionPolicy Bypass -File $Ctl status
if (-not $ok) { Die "the server did not start. See: powershell -File `"$Ctl`" logs" }
Say ''
Say 'Other devices reach the server only if Windows Firewall lets them. In PowerShell opened with "Run as administrator":'
Say '  New-NetFirewallRule -DisplayName Switchboard -Direction Inbound -Protocol TCP -LocalPort 8130,8131 -Action Allow -Profile Private'

# ---------------------------------------------------------------- first-run setup
if (-not $NoSetup -and [Environment]::UserInteractive -and -not [Console]::IsInputRedirected) {
    Step 'First-run setup'
    & $Py (Join-Path $App 'install\setup.py')
} else {
    Say ''
    Say "Next: run  powershell -File `"$Ctl`" setup  to pick the server's address and add your machines."
}
