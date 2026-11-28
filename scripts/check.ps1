param(
    [switch]$Fast
)
$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$megRs    = Join-Path $repoRoot "meg-rs"

function Invoke-Step {
    param([string]$Label, [string]$WorkDir, [string]$Exe, [string[]]$ArgList)
    Write-Host "==> $Label" -ForegroundColor Cyan
    Push-Location $WorkDir
    try {
        & $Exe @ArgList
        if ($LASTEXITCODE -ne 0) {
            throw "$Label failed (exit $LASTEXITCODE)"
        }
    } finally {
        Pop-Location
    }
}

if ($Fast) {
    Invoke-Step -Label "Rust fast check (meg-core)" -WorkDir $megRs -Exe "cargo" -ArgList @("test", "--locked", "-p", "meg-core", "service_plane_is_always_pure_rust_native")
    Write-Host "[OK] Fast check passed." -ForegroundColor Green
    exit 0
}

Invoke-Step -Label "Strict zero-warning clippy" -WorkDir $megRs -Exe "cargo" -ArgList @("clippy", "--locked", "--no-deps", "--workspace", "--", "-D", "warnings")
Invoke-Step -Label "Rust workspace tests (--locked)" -WorkDir $megRs -Exe "cargo" -ArgList @("test", "--locked", "--workspace")
Invoke-Step -Label "Python offline eval suite" -WorkDir $repoRoot -Exe "python" -ArgList @("-m", "pytest", "meg-eval", "-q")
Write-Host "[OK] Full check passed." -ForegroundColor Green
