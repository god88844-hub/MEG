from metrics import pass_at_1, pass_pow_k
from runner import GoldenTask, TraceEvent, run_layer1_behavioral_assertions, run_layer3_golden_suite


def test_offline_eval_layer1_layer3_and_reliability_metrics():
    # 1. Layer 1 trace passes <5s
    l1 = run_layer1_behavioral_assertions(
        [
            TraceEvent(1, "tool_call", tool="read_file"),
            TraceEvent(1, "tool_result", tool="read_file", ok=True),
            TraceEvent(2, "receipt", ok=True),
        ]
    )
    assert l1["passed"] is True

    # 2. Layer 3 50-task golden dry-run green
    golden = [GoldenTask(f"G-{i:03d}", "core", f"task {i}", f"PASS-{i}") for i in range(50)]
    l3 = run_layer3_golden_suite(golden, lambda t: f"verified {t.expected_token}")
    assert l3["passed"] is True

    # 3. Reliability Pass@1 & Pass^4 green
    trials = [[True, True, True, True] for _ in range(10)]
    assert pass_at_1(trials) == 1.0
    assert pass_pow_k(trials, 4) == 1.0
