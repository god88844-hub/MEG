"""Verify the 8-task live `meg.exe` Aegis-Tier Offline Suite & live benchmark catalog."""

from benchmark_suite import run_aegis_benchmark_suite
from run_live_bench import build_live_bench_catalog


def test_aegis_live_meg_exe_eight_task_benchmark_suite(tmp_path):
    report = run_aegis_benchmark_suite(tmp_path)
    assert report["passed"] == 8, report
    assert report["total"] == 8, report
    assert report["pass_at_1"] == 1.0, report
    assert report["false_completion_rate"] == 0.0, report
    assert report["security_violations"] == 0, report


def test_live_bench_catalog_has_coding_and_security_oracles():
    tasks = build_live_bench_catalog()
    assert len(tasks) == 5
    assert sum(1 for t in tasks if t.kind == "coding") == 4
    assert sum(1 for t in tasks if t.kind == "security") == 1
    assert all(t.verify_cmd.strip() for t in tasks)