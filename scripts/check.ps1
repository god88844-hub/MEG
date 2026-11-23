param(
    [switch]$Fast,
    [switch]$SkipRelease,
    [switch]$Clippy
)

$ErrorActionPreference = "Stop"
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

$repoRoot = Split-Path -Parent $PSScriptRoot
$megRs    = Join-Path $repoRoot "meg-rs"
$megPlane = Join-Path $repoRoot "meg-plane"
$megEval  = Join-Path $repoRoot "meg-eval"

function Invoke-Step {
    param(
        [string]$Label,
        [string]$WorkDir,
        [string]$Exe,
        [string[]]$ArgList
    )
    Write-Host "==> $Label" -ForegroundColor Cyan
    Push-Location $WorkDir
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $Exe @ArgList
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
        Pop-Location
    }
    if ($code -ne 0) {
        Write-Host "[FAIL] $Label (exit $code)" -ForegroundColor Red
        exit $code
    }
    Write-Host "[PASS] $Label" -ForegroundColor Green
}

if ($Fast) {
    Invoke-Step -Label "Rust fast check (meg-core lib)" -WorkDir $megRs -Exe "cargo" -ArgList @("test", "--locked", "-p", "meg-core", "--lib", "truncate::tests::short_text_untouched")
    Invoke-Step -Label "Bun fast check (plane.test.ts)" -WorkDir $megPlane -Exe "bun" -ArgList @("test", "test/plane.test.ts")
    Write-Host "Fast check PASSED." -ForegroundColor Green
    exit 0
}

Invoke-Step -Label "Rust workspace tests (--locked)" -WorkDir $megRs -Exe "cargo" -ArgList @("test", "--locked", "--workspace")
if ($Clippy) {
    Invoke-Step -Label "Rust clippy (--locked --workspace)" -WorkDir $megRs -Exe "cargo" -ArgList @("clippy", "--locked", "--workspace", "--all-targets")
}
if (-not $SkipRelease) {
    Invoke-Step -Label "Rust release build (--locked)" -WorkDir $megRs -Exe "cargo" -ArgList @("build", "--locked", "--release", "--workspace")
}
Invoke-Step -Label "Bun control-plane tests" -WorkDir $megPlane -Exe "bun" -ArgList @("test", "--timeout", "20000")
Invoke-Step -Label "Bun control-plane bundle" -WorkDir $megPlane -Exe "bun" -ArgList @("build", "src/index.ts", "--target=bun", "--outfile=dist/plane.js")

if (-not $SkipRelease) {
    $megExe = Join-Path $megRs "target\release\meg.exe"
    if (Test-Path $megExe) {
        Invoke-Step -Label "Release binary smoke (--version)" -WorkDir $repoRoot -Exe $megExe -ArgList @("--version")
    }
}

if ((Test-Path $megEval) -and @(Get-ChildItem -Path $megEval -Recurse -Filter "test_*.py" -ErrorAction SilentlyContinue).Count -gt 0) {
    Invoke-Step -Label "Python offline evals (pytest)" -WorkDir $repoRoot -Exe "python" -ArgList @("-m", "pytest", "meg-eval")
}

Write-Host "Phase Gate check PASSED." -ForegroundColor Green
exit 0