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

def test_exact_dpp_logdet_soft_hurt_and_skill_adam():
    import math
    from evolve import SkillAdamOptimizer, compute_soft_hurt_objective, log_determinant, select_dpp_gram_coreset

    # Exact log det of 2x2 diagonal matrix [[2, 0], [0, 3]] == ln(6)
    assert math.isclose(log_determinant([[2.0, 0.0], [0.0, 3.0]], jitter=0.0), math.log(6.0), rel_tol=1e-6)

    # DPP Gram selector picks orthogonal basis vectors over duplicates
    ids = ["orth_x", "dup_x", "orth_y", "orth_z"]
    feats = [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
    quals = [1.0, 1.0, 1.0, 1.0]
    chosen, score = select_dpp_gram_coreset(ids, feats, quals, k=3, pinned_indices=[0])
    assert set(chosen) == {"orth_x", "orth_y", "orth_z"}
    assert score > -1e-5

    # Exact GUARD Soft-HURT penalty
    ok = compute_soft_hurt_objective(gain_pp=4.0, forgetting_pp=0.0)
    assert ok["admitted"] is True and ok["soft_hurt_score"] == 4.0
    bad = compute_soft_hurt_objective(gain_pp=4.0, forgetting_pp=1.0, lambda_hurt=10.0)
    assert bad["admitted"] is False and bad["soft_hurt_score"] == -6.0

    # Exact SkillAdam bias-corrected update (at t=1, m_hat=g, v_hat=g^2 => delta ~= lr * sign(g))
    opt = SkillAdamOptimizer(lr=0.1)
    w1 = opt.step({"skill_a": 1.0, "skill_b": 1.0}, {"skill_a": 2.0, "skill_b": -2.0})
    assert math.isclose(w1["skill_a"], 1.1, abs_tol=1e-4)
    assert math.isclose(w1["skill_b"], 0.9, abs_tol=1e-4)