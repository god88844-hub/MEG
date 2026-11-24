from evolve import CandidateMutation, run_controlled_evolution_dry_run
from metrics import pass_at_1, pass_pow_k
from runner import GoldenTask, TraceEvent, run_layer1_behavioral_assertions, run_layer3_golden_suite


def test_phase7_gate_offline_eval_and_candidate_pr_dry_run_zero_gaming_zero_regressions():
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

    # 4. Candidate evolution dry-run produces unmerged PR with CLEAN verdict & 0.00 pp forgetting
    mutation = CandidateMutation(
        candidate_id="phase7-dry-run",
        target_modules=("meg-plane/src/context.ts",),
        diff_summary="refine tail anchor",
        diff_text="+const TAIL_INTERVAL = 4;",
        token_cost=250,
        token_cap=2000,
        tier="Lite",
    )
    evo = run_controlled_evolution_dry_run(
        mutation,
        baseline_task_pass={"t1": True, "t2": True, "t3": False},
        candidate_task_pass={"t1": True, "t2": True, "t3": True},
        turn0_checklist_complete=True,
        fires_on_stable=False,
        three_config_results=[True, True, True],
        previous_peak_score=0.66,
    )
    assert evo["admitted"] is True
    assert evo["verdict"] == "CLEAN"
    assert evo["guard"]["forgetting_pp"] == 0.0
    assert evo["pr"].merged is False