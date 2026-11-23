param(
    [switch]$AllowDirty
)

$ErrorActionPreference = "Stop"
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}

$repoRoot = Split-Path -Parent $PSScriptRoot
$failures = New-Object System.Collections.Generic.List[string]

function Invoke-ToolCheck {
    param(
        [string]$Exe,
        [string[]]$ArgList
    )
    $cmd = Get-Command $Exe -ErrorAction SilentlyContinue
    if (-not $cmd) {
        return @{ Ok = $false; Output = "not found on PATH" }
    }
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $raw = & $Exe @ArgList 2>&1
        $code = $LASTEXITCODE
    } catch {
        $raw = $_.Exception.Message
        $code = 1
    } finally {
        $ErrorActionPreference = $prev
    }
    $text = (($raw | ForEach-Object { "$_" }) -join " ").Trim()
    return @{ Ok = ($code -eq 0 -and $text.Length -gt 0); Output = $text }
}

Write-Host "MEG Preflight Check ($repoRoot)"

# 1. Core toolchains: cargo, bun, git
foreach ($spec in @(
    @{ Name = "cargo"; Exe = "cargo"; Args = @("--version") },
    @{ Name = "bun";   Exe = "bun";   Args = @("--version") },
    @{ Name = "git";   Exe = "git";   Args = @("--version") }
)) {
    $res = Invoke-ToolCheck -Exe $spec.Exe -ArgList $spec.Args
    if ($res.Ok) {
        Write-Host ("  [OK]   {0,-8} {1}" -f $spec.Name, $res.Output)
    } else {
        Write-Host ("  [FAIL] {0,-8} {1}" -f $spec.Name, $res.Output) -ForegroundColor Red
        $failures.Add("$($spec.Name): $($res.Output)")
    }
}

# 2. Python 3.11+ (try python, python3, py -3)
$pyOk = $false
$pyOut = "not found on PATH"
foreach ($cand in @(
    @{ Exe = "python";  Args = @("--version") },
    @{ Exe = "python3"; Args = @("--version") },
    @{ Exe = "py";      Args = @("-3", "--version") }
)) {
    $res = Invoke-ToolCheck -Exe $cand.Exe -ArgList $cand.Args
    if ($res.Ok -and $res.Output -match 'Python\s+(\d+)\.(\d+)') {
        $major = [int]$Matches[1]
        $minor = [int]$Matches[2]
        $pyOut = $res.Output
        if ($major -gt 3 -or ($major -eq 3 -and $minor -ge 11)) {
            $pyOk = $true
        } else {
            $pyOut = "$($res.Output) (requires Python 3.11+)"
        }
        break
    }
}
if ($pyOk) {
    Write-Host ("  [OK]   {0,-8} {1}" -f "python", $pyOut)
} else {
    Write-Host ("  [FAIL] {0,-8} {1}" -f "python", $pyOut) -ForegroundColor Red
    $failures.Add("python: $pyOut")
}

# 3. Expected repo files
foreach ($rel in @("AGENTS.md", "docs\PLAN.md", "docs\STATE.md", "meg-rs\Cargo.toml", "meg-plane\package.json")) {
    $full = Join-Path $repoRoot $rel
    if (-not (Test-Path $full)) {
        Write-Host ("  [FAIL] missing  {0}" -f $rel) -ForegroundColor Red
        $failures.Add("missing $rel")
    }
}

# 4. Clean working tree check
$prev = $ErrorActionPreference
$ErrorActionPreference = "Continue"
$statusLines = @(& git -C $repoRoot status --porcelain 2>&1 | ForEach-Object { "$_" } | Where-Object { $_.Trim().Length -gt 0 })
$gitCode = $LASTEXITCODE
$ErrorActionPreference = $prev

if ($gitCode -ne 0) {
    Write-Host "  [FAIL] git      status failed" -ForegroundColor Red
    $failures.Add("git status failed")
} elseif ($statusLines.Count -gt 0) {
    if ($AllowDirty) {
        Write-Host ("  [WARN] tree     dirty ({0} path(s); allowed via -AllowDirty)" -f $statusLines.Count) -ForegroundColor Yellow
    } else {
        Write-Host ("  [FAIL] tree     working tree is not clean ({0} path(s))" -f $statusLines.Count) -ForegroundColor Red
        $statusLines | Select-Object -First 10 | ForEach-Object { Write-Host "         $_" -ForegroundColor Red }
        $failures.Add("working tree not clean")
    }
} else {
    Write-Host "  [OK]   tree     clean working tree"
}

if ($failures.Count -gt 0) {
    Write-Host ("Preflight FAILED ({0} issue(s))." -f $failures.Count) -ForegroundColor Red
    exit 1
}

Write-Host "Preflight PASSED." -ForegroundColor Green
exit 0