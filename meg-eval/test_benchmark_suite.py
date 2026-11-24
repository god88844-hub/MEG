"""Verify the live multi-task `meg.exe` Aegis-Tier Offline Benchmark Suite."""

from benchmark_suite import run_aegis_benchmark_suite


def test_aegis_live_meg_exe_five_task_benchmark_suite(tmp_path):
    report = run_aegis_benchmark_suite(tmp_path)
    assert report["passed"] == 5, report
    assert report["pass_at_1"] == 1.0, report
    assert report["false_completion_rate"] == 0.0, report
    assert report["security_violations"] == 0, report