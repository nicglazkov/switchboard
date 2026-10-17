# switchboard.ps1: start, stop and check the Switchboard server on this Windows PC.
# install-server.ps1 puts it in %LOCALAPPDATA%\Switchboard. Usage: switchboard.ps1 start|stop|restart|status|logs|setup
param([string]$Verb = 'status', [Parameter(ValueFromRemainingArguments = $true)] $Rest)
$ErrorActionPreference = 'Stop'

$Base = Join-Path $env:LOCALAPPDATA 'Switchboard'
$App = Join-Path $Base 'app'
$Logs = Join-Path $Base 'logs'
$Tasks = @('SwitchboardServer', 'SwitchboardDownloads')
$Py = (Get-Content (Join-Path $Base 'python.txt') -ErrorAction SilentlyContinue | Select-Object -First 1)

# The processes, found by what they run (the logon tasks only start them, through a hidden script).
function Procs {
    Get-CimInstance Win32_Process -Filter "Name = 'python.exe' OR Name = 'pythonw.exe'" |
        Where-Object { $_.CommandLine -and ($_.CommandLine -like "*$App\server\server.py*" -or $_.CommandLine -like '*http.server 8131*') }
}

function Up([string]$url) {
    try { Invoke-WebRequest -UseBasicParsing -Uri $url -TimeoutSec 3 | Out-Null; 'running' } catch { 'not answering' }
}

function Start-Parts { foreach ($t in $Tasks) { schtasks /Run /TN $t | Out-Null } }
function Stop-Parts { Procs | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue } }

function Status {
    Write-Host 'Switchboard on this PC'
    Write-Host "  server     http://127.0.0.1:8130  $(Up 'http://127.0.0.1:8130/api/summary')"
    Write-Host '  hub        not available on Windows yet (terminals and linked machines are off; machines are added over ssh)'
    Write-Host "  downloads  http://127.0.0.1:8131  $(Up 'http://127.0.0.1:8131/node/SHA256SUMS')"
}

switch ($Verb) {
    'start' { Start-Parts; Start-Sleep 3; Status }
    'stop' { Stop-Parts; Write-Host 'Stopped.' }
    'restart' { Stop-Parts; Start-Sleep 1; Start-Parts; Start-Sleep 3; Status }
    'status' { Status }
    'logs' { foreach ($p in 'server', 'downloads') { $f = Join-Path $Logs "$p.log"; if (Test-Path $f) { "== $p"; Get-Content $f -Tail 30 } } }
    'setup' { & $Py (Join-Path $App 'install\setup.py') @Rest }
    default { Write-Host 'usage: switchboard.ps1 start|stop|restart|status|logs|setup'; exit 2 }
}
