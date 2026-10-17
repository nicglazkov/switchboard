# Runs ON the Windows build host (called by scripts/build-windows.sh). Unpacks the source tarball, builds
# Switchboard.exe and the installer, checks both are GUI-subsystem exes with their icon, version info and manifest,
# and that neither needs the Visual C++ runtime DLLs (the CRT is linked in, see native/.cargo/config.toml). Then:
#   dist\Switchboard-Setup-<version>.exe   the installer with the app attached (switchboard-install --make-setup)
#   dist\Switchboard-windows.zip           Switchboard.exe and THIRD-PARTY-NOTICES.txt, for the in-app updater
#   dist\symbols\<version>-<build>\        the .pdb files with their exes, outside both downloads
# Signs only when SB_WIN_SIGN_CERT is set (see the marked block). Never launches the app. Windows PowerShell 5.1
# compatible.
param([string]$Root = "$env:USERPROFILE\src\switchboard-native", [string]$Tarball = "$Root.tar.gz")

$ErrorActionPreference = 'Continue'
$root = $Root -replace '/', '\'
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

function Fail([string]$msg) { Write-Output "windows: FAILED: $msg"; exit 1 }

# Fresh sources every time; keep target\ (incremental cache) and dist\.
New-Item -ItemType Directory -Force -Path $root | Out-Null
Get-ChildItem -LiteralPath $root -Force | Where-Object { $_.Name -ne 'target' -and $_.Name -ne 'dist' } |
    Remove-Item -Recurse -Force
tar -xzf $Tarball -C $root
if ($LASTEXITCODE -ne 0) { Fail 'tar -xzf' }
Remove-Item -LiteralPath $Tarball -Force

Set-Location $root
# The build id the app compares with the download page's latest.json ("Restart to update").
if (-not $env:SB_BUILD) { $env:SB_BUILD = (Get-Date).ToUniversalTime().ToString('yyyyMMddHHmmss') }
$ver = (Select-String -LiteralPath (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$sw = [System.Diagnostics.Stopwatch]::StartNew()
# Any strip on MSVC also drops the .pdb; the exe holds no debug info anyway (Cargo.toml's release profile).
$env:CARGO_PROFILE_RELEASE_STRIP = 'none'
cargo build --release --locked -q -p sb-desktop -p sb-install
if ($LASTEXITCODE -ne 0) { Fail 'cargo build' }
$secs = [int]$sw.Elapsed.TotalSeconds

$exe = Join-Path $root 'target\release\Switchboard.exe'
$inst = Join-Path $root 'target\release\switchboard-install.exe'
foreach ($f in $exe, $inst) { if (-not (Test-Path $f)) { Fail "missing $f" } }

# Resources: load as a data file only (no code runs) and count icon groups and version info.
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class SbRes {
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr LoadLibraryEx(string file, IntPtr reserved, uint flags);
    [DllImport("kernel32.dll")]
    static extern bool FreeLibrary(IntPtr h);
    delegate bool EnumProc(IntPtr h, IntPtr type, IntPtr name, IntPtr param);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern bool EnumResourceNames(IntPtr h, IntPtr type, EnumProc cb, IntPtr param);
    public static int Count(string path, int type) {
        IntPtr h = LoadLibraryEx(path, IntPtr.Zero, 0x2 | 0x20);
        if (h == IntPtr.Zero) return -1;
        int n = 0;
        EnumResourceNames(h, (IntPtr)type, (a, b, c, d) => { n++; return true; }, IntPtr.Zero);
        FreeLibrary(h);
        return n;
    }
}
'@

# One exe's checks: GUI subsystem, icon, version info with the product name and publisher, GPUI's manifest
# (PerMonitorV2 DPI, asInvoker so Windows never asks for admin), and no Visual C++ runtime DLL imports.
function Check([string]$path, [int]$minIcons) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    # PE subsystem: e_lfanew at 0x3C; Subsystem is at optional header + 68 = e_lfanew + 24 + 68.
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $subsystem = [BitConverter]::ToUInt16($bytes, $pe + 92)
    if ($subsystem -ne 2) { Fail "$path subsystem is $subsystem, want 2 (WINDOWS GUI)" }
    $groups = [SbRes]::Count($path, 14)   # RT_GROUP_ICON
    $icons = [SbRes]::Count($path, 3)     # RT_ICON
    $version = [SbRes]::Count($path, 16)  # RT_VERSION
    $manifest = [SbRes]::Count($path, 24) # RT_MANIFEST
    if ($groups -lt 1 -or $icons -lt $minIcons) { Fail "$path icon resources missing (groups=$groups icons=$icons)" }
    if ($version -lt 1) { Fail "$path has no version info" }
    if ($manifest -lt 1) { Fail "$path has no manifest (no PerMonitorV2 DPI awareness)" }
    $vi = (Get-Item $path).VersionInfo
    if (-not $vi.ProductName -or -not $vi.CompanyName -or -not $vi.FileVersion) {
        Fail "$path version info is missing ProductName, CompanyName or FileVersion"
    }
    # Import names are plain ASCII in the file; a static CRT leaves none of these.
    $ascii = [System.Text.Encoding]::ASCII.GetString($bytes)
    if ($ascii -match '(?i)(vcruntime1\d\d|msvcp1\d\d)(_\d)?\.dll') { Fail "$path needs $($Matches[0]) (the CRT is not static)" }
    return "subsystem=$subsystem icon_groups=$groups icons=$icons manifest=$manifest product='$($vi.ProductName)' company='$($vi.CompanyName)' file_version=$($vi.FileVersion)"
}
$appCheck = Check $exe 7
$instCheck = Check $inst 7

# ---- Signing: runs only when SB_WIN_SIGN_CERT is set ----------------------------------------------------------
# SB_WIN_SIGN_CERT      a .pfx file on this build host
# SB_WIN_SIGN_PASSWORD  its password (optional)
# SB_WIN_SIGN_TIMESTAMP an RFC 3161 timestamp server URL (optional; without one the signature ends with the cert)
# SB_WIN_SIGNTOOL       signtool.exe's path (optional; else the one on the PATH)
# The app and the plain installer are signed before Setup is made from them, then Setup itself.
function Sign([string]$path) {
    if (-not $env:SB_WIN_SIGN_CERT) { return }
    $tool = if ($env:SB_WIN_SIGNTOOL) { $env:SB_WIN_SIGNTOOL } else { 'signtool.exe' }
    $a = @('sign', '/fd', 'sha256', '/f', $env:SB_WIN_SIGN_CERT)
    if ($env:SB_WIN_SIGN_PASSWORD) { $a += @('/p', $env:SB_WIN_SIGN_PASSWORD) }
    if ($env:SB_WIN_SIGN_TIMESTAMP) { $a += @('/tr', $env:SB_WIN_SIGN_TIMESTAMP, '/td', 'sha256') }
    & $tool @a $path | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail "signtool could not sign $path" }
    Write-Output "signed: $(Split-Path -Leaf $path)"
}
Sign $exe
Sign $inst
# ---- end of signing ---------------------------------------------------------------------------------------------

New-Item -ItemType Directory -Force -Path (Join-Path $root 'dist') | Out-Null
$notices = Join-Path $root 'packaging\THIRD-PARTY-NOTICES.txt'

# The plain zip, for the in-app updater.
$zip = Join-Path $root 'dist\Switchboard-windows.zip'
if (Test-Path $zip) { Remove-Item -LiteralPath $zip -Force }
Compress-Archive -LiteralPath $exe, $notices -DestinationPath $zip -CompressionLevel Optimal
Set-Content -LiteralPath (Join-Path $root 'dist\Switchboard-windows.zip.build') -Value $env:SB_BUILD -NoNewline

# Setup: the installer with the app attached. Piping makes PowerShell wait for a window-subsystem exe.
Get-ChildItem -LiteralPath (Join-Path $root 'dist') -Filter 'Switchboard-Setup-*.exe' | Remove-Item -Force
$setup = Join-Path $root "dist\Switchboard-Setup-$ver.exe"
$made = & $inst --make-setup $setup $exe $notices 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or -not (Test-Path $setup)) { Fail "--make-setup: $made" }
Sign $setup
$said = (& $setup --version 2>&1 | Out-String).Trim()
if ($said -notmatch [regex]::Escape($ver)) { Fail "Setup --version said '$said'" }

$exeSize = (Get-Item $exe).Length
$zipSize = (Get-Item $zip).Length
$setupSize = (Get-Item $setup).Length
# switchboard-node for Add a machine.
cargo build --release --locked -q -p sb-node
if ($LASTEXITCODE -ne 0) { Fail 'cargo build sb-node' }
Copy-Item -LiteralPath (Join-Path $root 'target\release\switchboard-node.exe') -Destination (Join-Path $root 'dist\switchboard-node.exe') -Force

# Symbols for reading crash backtraces (docs/building.md), with the exes they match. Only this build's are kept here;
# build-windows.sh copies them. The linker names a .pdb after the crate's binary with dashes as underscores.
$symRoot = Join-Path $root 'dist\symbols'
if (Test-Path $symRoot) { Remove-Item -LiteralPath $symRoot -Recurse -Force }
$sym = Join-Path $symRoot "$ver-$env:SB_BUILD"
New-Item -ItemType Directory -Force -Path $sym | Out-Null
foreach ($b in 'Switchboard', 'switchboard-node', 'switchboard-install') {
    $pdb = Join-Path $root ("target\release\" + ($b -replace '-', '_') + '.pdb')
    if (-not (Test-Path $pdb)) { Fail "missing $pdb" }
    Copy-Item -LiteralPath $pdb -Destination $sym -Force
    Copy-Item -LiteralPath (Join-Path $root "target\release\$b.exe") -Destination $sym -Force
}
Write-Output "symbols: dist\symbols\$ver-$env:SB_BUILD"
Write-Output "node windows: $((Get-Item (Join-Path $root 'dist\switchboard-node.exe')).Length) bytes"
Write-Output "app: $appCheck"
Write-Output "installer: $instCheck"
Write-Output "setup: $(Split-Path -Leaf $setup) $setupSize bytes ($($made.Trim()))"
Write-Output "windows: v$ver build=${secs}s exe=$exeSize bytes zip=$zipSize bytes"
