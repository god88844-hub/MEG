# MEG Pure-Rust Single-Binary Installer (Windows x64)
# Usage: irm https://raw.githubusercontent.com/god88844-hub/MEG/main/scripts/install.ps1 | iex
$ErrorActionPreference = "Stop"
$repo = "god88844-hub/MEG"
$asset = "meg-x86_64-pc-windows-gnu.exe"
$installDir = Join-Path $env:USERPROFILE ".local\bin"
New-Item -ItemType Directory -Path $installDir -Force | Out-Null

$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
$tag = $release.tag_name
if (-not $tag) { throw "Could not resolve latest release tag for $repo" }
$binUrl = "https://github.com/$repo/releases/download/$tag/$asset"
$sumUrl = "https://github.com/$repo/releases/download/$tag/SHA256SUMS.txt"

$tmpBin = Join-Path $env:TEMP "meg-$tag.exe"
Write-Host "Downloading MEG $tag ($asset)..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $binUrl -OutFile $tmpBin -UseBasicParsing
    $sums = (Invoke-WebRequest -Uri $sumUrl -UseBasicParsing).Content
    if (-not $sums) {
        throw "SHA256SUMS.txt was empty; refusing unverified install"
    }
    $line = ($sums -split '\r?\n' | Where-Object { $_ -match ("(^|\s)" + [regex]::Escape($asset) + "\s*$") } | Select-Object -First 1)
    $expected = if ($line) { ($line.Trim() -split '\s+')[0] } else { $null }
    if (-not $expected -or $expected -notmatch '^[0-9a-fA-F]{64}$') {
        throw "Missing valid SHA-256 entry for $asset in SHA256SUMS.txt; refusing unverified install"
    }
    $actual = (Get-FileHash -Path $tmpBin -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected.ToLowerInvariant()) {
        throw "SHA-256 checksum mismatch! expected=$expected actual=$actual"
    }
    Write-Host "Verified SHA-256: $actual" -ForegroundColor Green
} catch {
    Remove-Item $tmpBin -Force -ErrorAction SilentlyContinue
    throw
}

$dest = Join-Path $installDir "meg.exe"
Move-Item -Path $tmpBin -Destination $dest -Force

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not ($userPath -split ';' | Where-Object { $_ -eq $installDir })) {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "Added $installDir to your User PATH (restart terminal to pick up)." -ForegroundColor Yellow
}

Write-Host "Installed MEG $tag -> $dest" -ForegroundColor Green
& $dest --version
