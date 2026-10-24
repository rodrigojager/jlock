<#
.SYNOPSIS
Installs jlock on Windows, from a GitHub Release.

    irm https://raw.githubusercontent.com/rodrigojager/jlock/main/install.ps1 | iex

.PARAMETER Version
The release to install, such as v0.1.0. Default: $env:JLOCK_VERSION, else the latest release.

.PARAMETER BaseUrl
Where the release files are. Default: $env:JLOCK_BASE_URL, else the project's GitHub Releases.
file:// works too (the tests use it).

.PARAMETER InstallDir
Where to put jlock.exe. Default: $env:JLOCK_INSTALL_DIR, else %LOCALAPPDATA%\Programs\jlock.

.PARAMETER NoPath
Do not add the folder to the user's PATH.
#>
param(
    [string]$Version = $env:JLOCK_VERSION,
    [string]$BaseUrl = $env:JLOCK_BASE_URL,
    [string]$InstallDir = $env:JLOCK_INSTALL_DIR,
    [switch]$NoPath
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = 'rodrigojager/jlock'
if (-not $BaseUrl) { $BaseUrl = "https://github.com/$Repo/releases/download" }
if (-not $InstallDir) { $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\jlock' }

function Say([string]$Text) { Write-Host "jlock install: $Text" }

# Get-FileHash and Expand-Archive come from modules that Windows PowerShell 5.1 cannot load when it
# was started from PowerShell 7 (PSModulePath points at 7's modules); .NET does the same work.
function Get-Sha256([string]$Path) {
    if (Get-Command Get-FileHash -ErrorAction SilentlyContinue) {
        return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLower()
    }
    $sha = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($Path)
    try { return (($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') }) -join '') }
    finally { $stream.Dispose(); $sha.Dispose() }
}

function Expand-Zip([string]$Zip, [string]$Dest) {
    if (Get-Command Expand-Archive -ErrorAction SilentlyContinue) {
        Expand-Archive -LiteralPath $Zip -DestinationPath $Dest -Force
        return
    }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [IO.Compression.ZipFile]::ExtractToDirectory($Zip, $Dest)
}

function Get-Remote([string]$Url, [string]$Dest) {
    if ($Url -like 'file://*') {
        Copy-Item -LiteralPath ([Uri]$Url).LocalPath -Destination $Dest
        return
    }
    # Windows PowerShell 5.1 may still default to TLS 1.0, which GitHub refuses.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing
}

# There is one Windows build, x64; Windows on ARM64 runs it through emulation.
switch ($env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { $Target = 'x86_64-pc-windows-msvc' }
    'ARM64' { $Target = 'x86_64-pc-windows-msvc' }
    default { throw "jlock install: no build for this architecture: $($env:PROCESSOR_ARCHITECTURE)" }
}

if (-not $Version) {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
    $Version = $latest.tag_name
    if (-not $Version) { throw 'jlock install: no release found; pass -Version' }
}
if (-not $Version.StartsWith('v')) { $Version = "v$Version" }
$Number = $Version.Substring(1)
$Asset = "jlock-$Number-$Target.zip"

$Temp = Join-Path ([IO.Path]::GetTempPath()) ("jlock-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Temp | Out-Null
try {
    Say "downloading $Asset ($Version)"
    $Zip = Join-Path $Temp $Asset
    $Sums = Join-Path $Temp 'SHA256SUMS'
    Get-Remote "$BaseUrl/$Version/$Asset" $Zip
    Get-Remote "$BaseUrl/$Version/SHA256SUMS" $Sums

    $Want = $null
    foreach ($line in Get-Content -LiteralPath $Sums) {
        $parts = $line -split '\s+', 2
        if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $Asset) { $Want = $parts[0].ToLower() }
    }
    if (-not $Want) { throw "jlock install: $Asset is not listed in SHA256SUMS; nothing was installed" }
    $Got = Get-Sha256 $Zip
    if ($Got -ne $Want) {
        throw "jlock install: checksum mismatch for $Asset (expected $Want, got $Got); nothing was installed"
    }

    $Unpacked = Join-Path $Temp 'unpacked'
    Expand-Zip $Zip $Unpacked
    $New = Join-Path $Unpacked 'jlock.exe'
    if (-not (Test-Path -LiteralPath $New)) { throw "jlock install: $Asset has no jlock.exe" }

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $Exe = Join-Path $InstallDir 'jlock.exe'
    $Old = "$Exe.old"
    if (Test-Path -LiteralPath $Exe) {
        # A running jlock.exe cannot be overwritten, but it can be renamed.
        Remove-Item -LiteralPath $Old -Force -ErrorAction SilentlyContinue
        Move-Item -LiteralPath $Exe -Destination $Old -Force
    }
    Copy-Item -LiteralPath $New -Destination $Exe
    Remove-Item -LiteralPath $Old -Force -ErrorAction SilentlyContinue

    $Installed = & $Exe --version
    if ($LASTEXITCODE -ne 0) { throw "jlock install: the installed $Exe does not run" }
    Say "installed $Installed in $InstallDir"

    if (-not $NoPath) {
        $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $Entries = @($UserPath -split ';' | Where-Object { $_ })
        $Have = $Entries | Where-Object { $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\') }
        if (-not $Have) {
            # Writes HKCU\Environment and tells Windows; terminals opened from now on see it.
            [Environment]::SetEnvironmentVariable('Path', (($Entries + $InstallDir) -join ';'), 'User')
            Say "added $InstallDir to your PATH; new terminals will find jlock"
        }
        if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\') })) {
            $env:Path = "$env:Path;$InstallDir"
        }
    }
    Say 'check it with: jlock doctor'
}
finally {
    Remove-Item -LiteralPath $Temp -Recurse -Force -ErrorAction SilentlyContinue
}
