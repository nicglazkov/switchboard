# Install the Switchboard agent on a Windows PC that runs Claude Code, and link it to your server.
# No admin rights needed (except to let the server in over ssh as an administrator; see -SshKey).
# Usually pasted from the server's setup, in PowerShell:
#
#   & ([scriptblock]::Create((iwr -UseBasicParsing http://<server>:8131/install-agent.ps1).Content)) -Server <server> -Code <code>
#
#   -Server ADDR    the Switchboard server (an address or name; its downloads are at http://ADDR:8131)
#   -Code CODE      link with this code from Add a machine (no ssh into this PC)
#   -SshKey "KEY"   let the server in over ssh with this public key, limited to the agent (no shell)
#   -Name ID        this PC's id (lowercase letters and digits; default: from its name)
#   -Label NAME     this PC's name in Switchboard (default: its own name)
#   -Sb PATH        use this sb.py instead of downloading one
#   -Node PATH      use this switchboard-node.exe instead of downloading one
#   -Update         replace an agent that is already here (by default it is kept)
param(
    [string]$Server = '',
    [string]$Code = '',
    [string]$SshKey = '',
    [string]$Name = '',
    [string]$Label = '',
    [string]$Sb = '',
    [string]$Node = '',
    [switch]$Update
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is many times slower with its progress bar

function Say([string]$m) { Write-Host $m }
function Die([string]$m) { Write-Host "install-agent: $m" -ForegroundColor Red; throw "install-agent: $m" }

if ($Server -and $Server -notmatch '^[A-Za-z0-9][A-Za-z0-9.:_-]*$|^\[[0-9A-Fa-f:.]+\]$') { Die '-Server takes an address or a name, like 10.0.0.5 or studio.local' }
if ($Code -and $Code -notmatch '^[A-Za-z0-9-]{4,20}$') { Die '-Code takes the code shown in Add a machine' }
if (($Code -or $SshKey) -and -not $Server -and -not $Node) { Die '-Code needs -Server (where to link)' }

# ---------------------------------------------------------------- Python
# The Microsoft Store puts a "python" that only opens the Store; ask each candidate for its version instead.
$Py = $null
foreach ($c in @('python', 'py')) {
    $cmd = Get-Command $c -ErrorAction SilentlyContinue
    if (-not $cmd) { continue }
    $args0 = @(); if ($c -eq 'py') { $args0 = @('-3') }
    try { $v = & $cmd.Source @args0 -c 'import sys; print("%d.%d" % sys.version_info[:2])' 2>$null } catch { $v = $null }
    if ($v -and [version]$v -ge [version]'3.9') {
        $Py = (& $cmd.Source @args0 -c 'import sys; print(sys.executable)').Trim()
        break
    }
}
if (-not $Py) {
    Say 'Python 3 is not installed. The agent is a Python program (3.9 or newer).'
    Say 'Install it with:  winget install -e --id Python.Python.3.12'
    Say '(or from https://www.python.org/downloads/, ticking "Add python.exe to PATH"), then open a new PowerShell and run this again.'
    return  # not exit: pasted as a script block, exit would close the person's PowerShell
}
Say "Python at $Py"

$Dl = if ($Server) { "http://${Server}:8131/node" } else { '' }
$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("switchboard-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $Tmp | Out-Null

function Fetch([string]$url, [string]$file) { Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $file -TimeoutSec 120 }
function SumsOk([string]$file, [string]$name) {
    if (-not $Dl) { return $true }
    try { $sums = (Invoke-WebRequest -UseBasicParsing -Uri "$Dl/SHA256SUMS" -TimeoutSec 30).Content } catch { Say '(no SHA256SUMS on the server; not checked)'; return $true }
    $want = $null
    foreach ($l in ($sums -split "`n")) {
        $p = $l.Trim() -split '\s+', 2
        if ($p.Count -eq 2 -and $p[1].TrimStart('*') -eq $name) { $want = $p[0] }
    }
    if (-not $want) { return $true }
    return (Get-FileHash -Algorithm SHA256 $file).Hash.ToLower() -eq $want.ToLower()
}

try {
    if (-not (Get-Command claude -ErrorAction SilentlyContinue) -and -not (Test-Path "$HOME\.local\bin\claude.exe")) {
        Say 'Claude Code is not installed here yet. Switchboard runs it, so install it too:'
        Say '  irm https://claude.ai/install.ps1 | iex'
        Say 'The agent goes in now and starts working once Claude Code is here.'
    }

    # ------------------------------------------------------------ the agent (sb.py)
    $SbDir = Join-Path $HOME '.claude\switchboard'
    $SbPath = Join-Path $SbDir 'sb.py'
    New-Item -ItemType Directory -Force $SbDir | Out-Null
    if ((Test-Path $SbPath) -and -not $Update) {
        Say "The agent is here already ($SbPath); keeping it. Use -Update to replace it."
    } else {
        $new = Join-Path $Tmp 'sb.py'
        $repoSb = if ($PSScriptRoot) { Join-Path $PSScriptRoot '..\..\agent\sb.py' } else { '' }
        if ($Sb) { Copy-Item $Sb $new }
        elseif ($repoSb -and (Test-Path $repoSb)) { Copy-Item $repoSb $new }
        elseif ($Dl) { try { Fetch "$Dl/sb.py" $new } catch { Die "could not download the agent from $Dl/sb.py. Is the server running?" } }
        else { Die 'where is the agent? Pass -Server (to download it) or -Sb PATH.' }
        if (-not (SumsOk $new 'sb.py')) { Die "the agent from $Dl did not match its checksum; nothing was changed." }
        & $Py -m py_compile $new
        if ($LASTEXITCODE -ne 0) { Die "the agent did not compile with $Py." }
        Copy-Item $new $SbPath -Force
        & $Py $SbPath install
        if ($LASTEXITCODE -ne 0) { Die "the agent's installer failed (see above)." }
        Say "Agent installed: Claude Code's hooks, status line and the switchboard MCP server are set up in $HOME\.claude."
    }

    # ------------------------------------------------------------ link with a code
    if ($Code) {
        $exe = Join-Path $Tmp 'switchboard-node.exe'
        if ($Node) { Copy-Item $Node $exe }
        else {
            try { Fetch "$Dl/switchboard-node.exe" $exe } catch { Die 'the server has no switchboard-node for Windows yet. Add this PC over ssh instead (see docs/install.md).' }
            if (-not (SumsOk $exe 'switchboard-node.exe')) { Die "switchboard-node.exe from $Dl did not match its checksum; nothing was changed." }
        }
        Unblock-File $exe -ErrorAction SilentlyContinue
        $pairArgs = @('pair', "${Server}:8132", $Code)
        if ($Name) { $pairArgs += @('--name', $Name) }
        if ($Label) { $pairArgs += @('--label', $Label) }
        & $exe @pairArgs
        if ($LASTEXITCODE -ne 0) { Die "linking failed (see above). Codes work once, for 10 minutes: get a new one with 'switchboard setup --code' on the server." }
        & $exe install
        if ($LASTEXITCODE -ne 0) { Die 'the link is saved, but the node could not be set to run at logon (see above).' }
    }

    # ------------------------------------------------------------ let the server in over ssh
    if ($SshKey) {
        if ($SshKey -notmatch '^(ssh-ed25519|ssh-rsa|ecdsa-sha2-\S+) [A-Za-z0-9+/=]+( \S+)?$') { Die '-SshKey takes a public key line (ssh-ed25519 ...), pasted exactly' }
        $body = ($SshKey -split ' ')[1]
        $line = "command=`"$($Py -replace '\\', '/') $($SbPath -replace '\\', '/') rpc`",no-port-forwarding,no-agent-forwarding,no-X11-forwarding $SshKey"
        # Windows' OpenSSH reads an administrator's keys from one shared file, and only an administrator can write it.
        $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) -or
                 ((whoami /groups) -match 'S-1-5-32-544')
        $file = if ($admin) { Join-Path $env:ProgramData 'ssh\administrators_authorized_keys' } else { Join-Path $HOME '.ssh\authorized_keys' }
        try {
            New-Item -ItemType Directory -Force (Split-Path $file) | Out-Null
            $keep = @(); if (Test-Path $file) { $keep = @(Get-Content $file | Where-Object { $_ -notmatch [regex]::Escape($body) }) }
            Set-Content -Path $file -Value (@($keep) + $line) -Encoding ascii
            if ($admin) { icacls $file /inheritance:r /grant 'Administrators:F' /grant 'SYSTEM:F' | Out-Null }
            Say 'The server can now reach the agent here over ssh (and nothing else).'
        } catch {
            Say "Could not write $file ($($_.Exception.Message))."
            Say 'This account is an administrator, so the key goes in that shared file: run this again in PowerShell opened with "Run as administrator".'
            throw
        }
        if (-not (Get-Service sshd -ErrorAction SilentlyContinue)) {
            Say 'The OpenSSH server is not installed. As an administrator, run:'
            Say '  Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0; Set-Service sshd -StartupType Automatic; Start-Service sshd'
        }
        Say "Back on the server, add it as $env:USERNAME@$env:COMPUTERNAME."
    }

    Say ''
    Say 'Done. This PC shows up in Switchboard within a minute.'
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
