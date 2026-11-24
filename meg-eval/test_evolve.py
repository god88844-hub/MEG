from pathlib import Path

from evolve import (
    CandidateMutation,
    CoresetTask,
    FailureCluster,
    ParetoCandidate,
    RejectionLedger,
    run_controlled_evolution_dry_run,
    select_dpp_10_task_coreset,
    select_pareto_frontier,
    update_clusters_with_decay,
)


def test_gated_modular_evolution_soft_hurt_and_unmerged_pr_only(tmp_path: Path):
    ledger = RejectionLedger(tmp_path / "rejections.jsonl")
    base = {"t1": True, "t2": True, "t3": False, "t4": False}

    # 1. Multi-module mutation rejected
    multi = CandidateMutation("c-multi", ("meg-plane/src/context.ts", "meg-plane/src/router.ts"), "two files", "+x", 500, 2000)
    r_multi = run_controlled_evolution_dry_run(
        multi,
        baseline_task_pass=base,
        candidate_task_pass={"t1": True, "t2": True, "t3": True, "t4": False},
        turn0_checklist_complete=True,
        fires_on_stable=False,
        three_config_results=[True, True, True],
        previous_peak_score=0.5,
        rejection_ledger=ledger,
    )
    assert r_multi["admitted"] is False
    assert "GATED_MODULAR_VIOLATION" in r_multi["reason"]

    # 2. Ring-0 live edit rejected
    ring0 = CandidateMutation("c-ring0", ("meg-rs/crates/meg-tools/src/security_guards.rs",), "ring0", "+x", 500, 2000)
    r_ring0 = run_controlled_evolution_dry_run(
        ring0,
        baseline_task_pass=base,
        candidate_task_pass={"t1": True, "t2": True, "t3": True, "t4": False},
        turn0_checklist_complete=True,
        fires_on_stable=False,
        three_config_results=[True, True, True],
        previous_peak_score=0.5,
        rejection_ledger=ledger,
    )
    assert r_ring0["admitted"] is False
    assert "RING0_LOCKED" in r_ring0["reason"]

    # 3. Soft-HURT rejects even 1 regressed task (> 0.00 pp forgetting)
    forget = CandidateMutation("c-forget", ("meg-plane/src/context.ts",), "regresses t1", "+trim", 500, 2000)
    r_forget = run_controlled_evolution_dry_run(
        forget,
        baseline_task_pass=base,
        candidate_task_pass={"t1": False, "t2": True, "t3": True, "t4": True},
        turn0_checklist_complete=True,
        fires_on_stable=False,
        three_config_results=[True, True, True],
        previous_peak_score=0.5,
        rejection_ledger=ledger,
    )
    assert r_forget["admitted"] is False
    assert r_forget["guard"]["forgetting_pp"] > 0.0

    # 4. Dual-Rubric Governance Gauntlet flags GAMING diff
    gaming = CandidateMutation("c-game", ("meg-plane/src/context.ts",), "cheat", "if task_id == 't3': return ok", 500, 2000)
    r_game = run_controlled_evolution_dry_run(
        gaming,
        baseline_task_pass=base,
        candidate_task_pass={"t1": True, "t2": True, "t3": True, "t4": False},
        turn0_checklist_complete=True,
        fires_on_stable=False,
        three_config_results=[True, True, True],
        previous_peak_score=0.5,
        rejection_ledger=ledger,
    )
    assert r_game["admitted"] is False
    assert r_game["verdict"] == "GAMING"

    # 5. Clean single-module improvement with 0.00 pp forgetting -> unmerged Draft PR only
    clean = CandidateMutation("c-clean", ("meg-plane/src/context.ts",), "tighten tail anchor", "+anchor", 600, 2000, tier="Med")
    r_clean = run_controlled_evolution_dry_run(
        clean,
        baseline_task_pass=base,
        candidate_task_pass={"t1": True, "t2": True, "t3": True, "t4": False},
        turn0_checklist_complete=True,
        fires_on_stable=False,
        three_config_results=[True, True, True],
        previous_peak_score=0.5,
        rejection_ledger=ledger,
    )
    assert r_clean["admitted"] is True
    assert r_clean["verdict"] == "CLEAN"
    pr = r_clean["pr"]
    assert pr.merged is False
    assert pr.auto_merge is False
    assert pr.requires_human_signoff is True
    assert pr.before_receipt["pass_rate"] == 0.5
    assert pr.after_receipt["pass_rate"] == 0.75


def test_oo_harness_clusters_dpp_10_coreset_and_pareto_frontier():
    clusters = [
        FailureCluster("c1", ["t1", "t2"], 1.0, "timeout"),
        FailureCluster("c2", ["t3", "t4"], 0.8, "edit_stale"),
    ]
    merged = update_clusters_with_decay(clusters, "merge", target_ids=("c1", "c2"))
    assert len(merged) == 1
    assert merged[0].priority == 0.85

    split = update_clusters_with_decay(merged, "split", target_ids=(merged[0].cluster_id,))
    assert len(split) == 2

    pool = [
        CoresetTask("f1", "auth", unanimous_fail=True, variance=0.0),
        CoresetTask("f2", "net", unanimous_fail=True, variance=0.0),
        CoresetTask("p1", "fs", unanimous_pass=True, variance=0.0),
        CoresetTask("p2", "cli", unanimous_pass=True, variance=0.0),
        *[CoresetTask(f"m{i}", f"cat{i}", variance=0.9 - i * 0.05) for i in range(1, 10)],
    ]
    coreset = select_dpp_10_task_coreset(pool)
    assert len(coreset) == 10
    assert sum(1 for t in coreset if t.unanimous_fail) == 2
    assert sum(1 for t in coreset if t.unanimous_pass) == 2

    frontier = select_pareto_frontier(
        [
            ParetoCandidate("lite-1", "Lite", pass_rate=0.80, cost_usd=0.20),
            ParetoCandidate("med-1", "Med", pass_rate=0.90, cost_usd=0.60),
            ParetoCandidate("dominated", "Heavy", pass_rate=0.85, cost_usd=1.20),
        ]
    )
    assert [c.candidate_id for c in frontier] == ["med-1", "lite-1"]