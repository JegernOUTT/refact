param(
    [string]$Version = $env:VERSION,
    [string]$Binary,
    [switch]$NoModifyPath,
    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($HOME)) {
    Write-Error "HOME is not set"
    exit 1
}

$RepoUrl = "https://github.com/JegernOUTT/refact"
$ApiUrl = "https://api.github.com/repos/JegernOUTT/refact"
$InstallDir = Join-Path $HOME ".refact\bin"
$ExecutableName = "refact.exe"
$InstallPath = Join-Path $InstallDir $ExecutableName

function Show-Usage {
    @"
Install Refact into `%USERPROFILE%\.refact\bin`.

Usage:
  install.ps1 [-Version <v>] [-Binary <path>] [-NoModifyPath]

Options:
  -Version <v>     Install a specific Refact version. VERSION env is also honored.
  -Binary <path>   Install from a local refact.exe binary instead of GitHub Releases.
  -NoModifyPath    Do not add ~/.refact/bin to the user PATH.
  -Help            Show this help.
"@ | Write-Host
}

function Fail([string]$Message) {
    Write-Error $Message
    exit 1
}

function Normalize-Version([string]$Value) {
    if ([string]::IsNullOrWhiteSpace($Value)) {
        Fail "version is empty"
    }
    if ($Value.StartsWith("engine/v")) {
        return $Value.Substring(8)
    }
    if ($Value.StartsWith("engine/")) {
        return $Value.Substring(7)
    }
    if ($Value.StartsWith("v")) {
        return $Value.Substring(1)
    }
    return $Value
}

function Get-LatestVersion {
    try {
        $release = Invoke-RestMethod -Uri "$ApiUrl/releases/latest" -Headers @{ Accept = "application/vnd.github+json" }
    } catch {
        Fail "could not resolve latest release from $ApiUrl/releases/latest: $($_.Exception.Message)"
    }
    if (-not $release.tag_name) {
        Fail "latest release response did not include tag_name"
    }
    return Normalize-Version ([string]$release.tag_name)
}

function Resolve-Version {
    if ([string]::IsNullOrWhiteSpace($Version) -or $Version -eq "latest") {
        return Get-LatestVersion
    }
    return Normalize-Version $Version
}

function Assert-Windows {
    $platform = [System.Environment]::OSVersion.Platform
    if ($platform -ne [System.PlatformID]::Win32NT -and $platform -ne [System.PlatformID]::Win32Windows) {
        Fail "install.ps1 supports Windows only. Use install.sh on Linux or macOS. Detected: $platform"
    }
}

function Get-TargetTriple {
    Assert-Windows
    $arch = $env:PROCESSOR_ARCHITECTURE
    if (-not [string]::IsNullOrWhiteSpace($env:PROCESSOR_ARCHITEW6432)) {
        $arch = $env:PROCESSOR_ARCHITEW6432
    }
    if ([string]::IsNullOrWhiteSpace($arch)) {
        Fail "could not detect CPU architecture"
    }

    switch ($arch.ToUpperInvariant()) {
        "AMD64" { return "x86_64-pc-windows-msvc" }
        "EM64T" { return "x86_64-pc-windows-msvc" }
        "X86" { return "i686-pc-windows-msvc" }
        "ARM64" { return "aarch64-pc-windows-msvc" }
        default { Fail "unsupported CPU architecture: $arch" }
    }
}

function Get-Sha256([string]$Path) {
    return (Get-FileHash -Algorithm SHA256 -Path $Path).Hash.ToLowerInvariant()
}

function Test-Sha256([string]$ArchivePath, [string]$ChecksumPath) {
    $firstLine = Get-Content -LiteralPath $ChecksumPath -TotalCount 1
    $line = if ($null -eq $firstLine) { "" } else { ([string]$firstLine).Trim() }
    if ([string]::IsNullOrWhiteSpace($line)) {
        Fail "checksum file is empty: $ChecksumPath"
    }
    $expected = ($line -split "\s+")[0].ToLowerInvariant()
    $actual = Get-Sha256 $ArchivePath
    if ($expected -ne $actual) {
        Fail "sha256 mismatch for $(Split-Path -Leaf $ArchivePath)"
    }
}

function Install-Binary([string]$SourcePath) {
    if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) {
        Fail "binary not found: $SourcePath"
    }
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $tempTarget = "$InstallPath.tmp.$PID"
    Copy-Item -LiteralPath $SourcePath -Destination $tempTarget -Force
    Move-Item -LiteralPath $tempTarget -Destination $InstallPath -Force
}

function Install-FromRelease {
    $resolvedVersion = Resolve-Version
    $target = Get-TargetTriple
    $archiveName = "refact-$resolvedVersion-$target.zip"
    $releaseBase = "$RepoUrl/releases/download/engine/v$resolvedVersion"
    $archiveUrl = "$releaseBase/$archiveName"
    $checksumUrl = "$archiveUrl.sha256"
    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "refact-install-$PID"
    $archivePath = Join-Path $tempDir $archiveName
    $checksumPath = Join-Path $tempDir "$archiveName.sha256"
    $extractDir = Join-Path $tempDir "extract"

    New-Item -ItemType Directory -Force -Path $extractDir | Out-Null
    try {
        Write-Host "Downloading $archiveUrl"
        Invoke-WebRequest -Uri $archiveUrl -OutFile $archivePath
        Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumPath
        Test-Sha256 $archivePath $checksumPath
        Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir -Force

        $candidate = Join-Path $extractDir $ExecutableName
        if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            $candidateItem = Get-ChildItem -LiteralPath $extractDir -Recurse -File -Filter $ExecutableName | Select-Object -First 1
            if ($null -eq $candidateItem) {
                Fail "archive did not contain $ExecutableName"
            }
            $candidate = $candidateItem.FullName
        }

        Install-Binary $candidate
    } finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force
        }
    }
}

function Add-ToUserPath {
    if ($NoModifyPath) {
        return $false
    }

    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    $segments = @()
    if (-not [string]::IsNullOrWhiteSpace($current)) {
        $segments = $current -split ";" | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    }

    foreach ($segment in $segments) {
        if ($segment.TrimEnd([char]92) -ieq $InstallDir.TrimEnd([char]92)) {
            return $false
        }
    }

    $newPath = if ([string]::IsNullOrWhiteSpace($current)) { $InstallDir } else { "$current;$InstallDir" }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    $env:Path = "$env:Path;$InstallDir"
    return $true
}

if ($Help) {
    Show-Usage
    exit 0
}

Assert-Windows

if (-not [string]::IsNullOrWhiteSpace($Binary)) {
    Install-Binary $Binary
} else {
    Install-FromRelease
}

$pathChanged = Add-ToUserPath

Write-Host "Refact installed successfully at $InstallPath"
if ($NoModifyPath) {
    Write-Host "PATH was not modified. Add $InstallDir to PATH to run refact from anywhere."
} elseif ($pathChanged) {
    Write-Host "Added $InstallDir to your user PATH. Restart your terminal before running refact."
} else {
    Write-Host "$InstallDir is already in your user PATH."
}
Write-Host "Start Refact with:"
Write-Host "  refact"
Write-Host "  refact tui"
Write-Host "  refact daemon"
Write-Host "Update Refact with:"
Write-Host "  refact self-update"
