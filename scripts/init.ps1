$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
Write-Host "MEG Pure-Rust Workspace Init Check" -ForegroundColor Cyan

foreach ($t in @(
    @{ Name = "cargo"; Exe = "cargo"; Args = @("--version") },
    @{ Name = "rustc"; Exe = "rustc"; Args = @("--version") },
    @{ Name = "git";   Exe = "git";   Args = @("--version") }
)) {
    $lines = @(& $t.Exe @($t.Args) 2>&1)
    if ($LASTEXITCODE -ne 0) { throw "$($t.Name) check failed" }
    Write-Host ("  [OK] {0,-6} {1}" -f $t.Name, $lines[0]) -ForegroundColor Green
}

foreach ($rel in @("AGENTS.md", "docs\STATE.md", "meg-rs\Cargo.toml", "meg-rs\crates\meg-core\skills\tdd-discipline\SKILL.md")) {
    $full = Join-Path $repoRoot $rel
    if (-not (Test-Path $full)) {
        throw "Missing expected file: $rel"
    }
    Write-Host ("  [OK] {0}" -f $rel) -ForegroundColor Green
}
Write-Host "Workspace ready (100% pure Rust; no Bun/Node required)." -ForegroundColor Green
exit 0
