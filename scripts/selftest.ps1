$ErrorActionPreference = "Stop"
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

$repoRoot  = Split-Path -Parent $PSScriptRoot
$checkPath = Join-Path $PSScriptRoot "check.ps1"
$targetRel = "meg-rs/crates/meg-core/src/services/mod.rs"
$targetAbs = Join-Path $repoRoot "meg-rs\crates\meg-core\src\services\mod.rs"

if (-not (Test-Path $targetAbs)) {
    Write-Host "[FAIL] Target file not found: $targetAbs" -ForegroundColor Red
    exit 1
}

$prev = $ErrorActionPreference
$ErrorActionPreference = "Continue"
$dirtyTarget = @(& git -C $repoRoot status --porcelain -- $targetRel 2>&1 | ForEach-Object { "$_" } | Where-Object { $_.Trim().Length -gt 0 })
$ErrorActionPreference = $prev
if ($dirtyTarget.Count -gt 0) {
    Write-Host "[FAIL] $targetRel has uncommitted changes; commit or stash before running selftest." -ForegroundColor Red
    exit 1
}

function Run-CheckFast {
    $p = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & powershell -NoProfile -ExecutionPolicy Bypass -File $checkPath -Fast | Out-Host
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $p
    }
}

Write-Host "Step 1/4: Verify clean baseline passes check.ps1 -Fast" -ForegroundColor Cyan
if ((Run-CheckFast) -ne 0) { exit 1 }

$origBytes = [System.IO.File]::ReadAllBytes($targetAbs)
$origText  = [System.Text.Encoding]::UTF8.GetString($origBytes)
$needle    = "matches!(self, Self::Native(_))"
$bug       = "false /* SELFTEST_DELTA */"

if (-not $origText.Contains($needle)) {
    Write-Host "[FAIL] Selftest anchor not found in $targetRel" -ForegroundColor Red
    exit 1
}

$bugCode = 0
try {
    Write-Host "Step 2/4: Plant synthetic bug delta in $targetRel" -ForegroundColor Cyan
    $mutated = $origText.Replace($needle, $bug)
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($targetAbs, $mutated, $utf8NoBom)

    Write-Host "Step 3/4: Prove check.ps1 -Fast fails non-zero on synthetic bug" -ForegroundColor Cyan
    $bugCode = Run-CheckFast
} finally {
    Write-Host "Step 4/4: Revert synthetic bug delta (exact byte restore)" -ForegroundColor Cyan
    [System.IO.File]::WriteAllBytes($targetAbs, $origBytes)
}

if ($bugCode -eq 0) {
    Write-Host "[FAIL] Selftest FAILED: check.ps1 exited 0 despite planted bug delta!" -ForegroundColor Red
    exit 1
}
Write-Host "[OK]   check.ps1 caught synthetic bug delta (exit $bugCode)" -ForegroundColor Green

if ((Run-CheckFast) -ne 0) { exit 1 }
Write-Host "Selftest PASSED (Green -> Red on delta -> Clean Revert -> Green)." -ForegroundColor Green
exit 0
