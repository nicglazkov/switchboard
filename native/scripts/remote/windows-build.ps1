# Runs ON the Windows build host (called by scripts/build-windows.sh). Unpacks the source tarball, builds
# Switchboard.exe, checks it is a GUI-subsystem exe with the icon embedded, zips it to
# dist\Switchboard-windows.zip (with THIRD-PARTY-NOTICES.txt next to the exe). The .pdb files go to
# dist\symbols\<version>-<build>\, outside the zip. Never launches the app. Windows PowerShell 5.1 compatible.
param([string]$Tarball = "$env:USERPROFILE\src\switchboard-native.tar.gz")

$ErrorActionPreference = 'Continue'
$root = Join-Path $env:USERPROFILE 'src\switchboard-native'
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
$sw = [System.Diagnostics.Stopwatch]::StartNew()
# Any strip on MSVC also drops the .pdb; the exe holds no debug info anyway (Cargo.toml's release profile).
$env:CARGO_PROFILE_RELEASE_STRIP = 'none'
cargo build --release --locked -q -p sb-desktop
if ($LASTEXITCODE -ne 0) { Fail 'cargo build' }
$secs = [int]$sw.Elapsed.TotalSeconds

$exe = Join-Path $root 'target\release\Switchboard.exe'
if (-not (Test-Path $exe)) { Fail "missing $exe" }

# PE subsystem: e_lfanew at 0x3C; Subsystem is at optional header + 68 = e_lfanew + 24 + 68.
$bytes = [System.IO.File]::ReadAllBytes($exe)
$pe = [BitConverter]::ToInt32($bytes, 0x3C)
$subsystem = [BitConverter]::ToUInt16($bytes, $pe + 92)
if ($subsystem -ne 2) { Fail "subsystem is $subsystem, want 2 (WINDOWS GUI)" }

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
$groups = [SbRes]::Count($exe, 14)   # RT_GROUP_ICON
$icons = [SbRes]::Count($exe, 3)     # RT_ICON
$version = [SbRes]::Count($exe, 16)  # RT_VERSION
$manifest = [SbRes]::Count($exe, 24) # RT_MANIFEST (GPUI's: PerMonitorV2 DPI, Windows 10+)
if ($groups -lt 1 -or $icons -lt 7) { Fail "icon resources missing (groups=$groups icons=$icons)" }
if ($manifest -lt 1) { Fail 'GPUI manifest missing (no PerMonitorV2 DPI awareness)' }

New-Item -ItemType Directory -Force -Path (Join-Path $root 'dist') | Out-Null
$zip = Join-Path $root 'dist\Switchboard-windows.zip'
if (Test-Path $zip) { Remove-Item -LiteralPath $zip -Force }
$notices = Join-Path $root 'packaging\THIRD-PARTY-NOTICES.txt'
Compress-Archive -LiteralPath $exe, $notices -DestinationPath $zip -CompressionLevel Optimal
Set-Content -LiteralPath (Join-Path $root 'dist\Switchboard-windows.zip.build') -Value $env:SB_BUILD -NoNewline

$exeSize = (Get-Item $exe).Length
$zipSize = (Get-Item $zip).Length
$fv = (Get-Item $exe).VersionInfo.FileVersion
# switchboard-node for Add a machine.
cargo build --release --locked -q -p sb-node
if ($LASTEXITCODE -ne 0) { Fail 'cargo build sb-node' }
Copy-Item -LiteralPath (Join-Path $root 'target\release\switchboard-node.exe') -Destination (Join-Path $root 'dist\switchboard-node.exe') -Force

# Symbols for reading crash backtraces (docs/building.md), with the exes they match. Only this build's are kept here;
# build-windows.sh copies them.
$ver = (Select-String -LiteralPath (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$symRoot = Join-Path $root 'dist\symbols'
if (Test-Path $symRoot) { Remove-Item -LiteralPath $symRoot -Recurse -Force }
$sym = Join-Path $symRoot "$ver-$env:SB_BUILD"
New-Item -ItemType Directory -Force -Path $sym | Out-Null
foreach ($b in 'Switchboard', 'switchboard-node') {
    $pdb = Join-Path $root "target\release\$b.pdb"
    if (-not (Test-Path $pdb)) { Fail "missing $pdb" }
    Copy-Item -LiteralPath $pdb -Destination $sym -Force
    Copy-Item -LiteralPath (Join-Path $root "target\release\$b.exe") -Destination $sym -Force
}
Write-Output "symbols: dist\symbols\$ver-$env:SB_BUILD"
Write-Output "node windows: $((Get-Item (Join-Path $root 'dist\switchboard-node.exe')).Length) bytes"
Write-Output "windows: v$fv subsystem=$subsystem icon_groups=$groups icons=$icons version_res=$version manifest=$manifest build=${secs}s exe=$exeSize bytes zip=$zipSize bytes"
