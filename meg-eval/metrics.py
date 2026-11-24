"""P7-02: Reliability, DORA & MAD-RSI Metrics (meg-eval/metrics.py):
- Pass@1, Pass^4, Flip rate, unbiased Pass@k (exact combinatorial estimator)
- CoP (Cost per Verified Outcome)
- Harness Constraint Effect
- Defect Escape Rate
- False Completion Rate
- MAD-RSI Pass^M and D_q
"""

from __future__ import annotations

import math
from typing import Dict, Sequence


def unbiased_pass_at_k(n: int, c: int, k: int) -> float:
    """Exact unbiased Pass@k estimator: 1 - comb(n - c, k) / comb(n, k)."""
    if n <= 0 or k <= 0 or k > n:
        raise ValueError(f"Invalid Pass@k parameters: n={n}, c={c}, k={k}")
    if c <= 0:
        return 0.0
    if n - c < k:
        return 1.0
    return 1.0 - (math.comb(n - c, k) / math.comb(n, k))


def pass_at_1(trial_matrix: Sequence[Sequence[bool]]) -> float:
    """Mean Pass@1 across tasks (each row is a list of trial outcomes for one task)."""
    if not trial_matrix:
        return 0.0
    scores = [unbiased_pass_at_k(len(row), sum(1 for x in row if x), 1) for row in trial_matrix if row]
    return sum(scores) / len(scores) if scores else 0.0


def pass_pow_k(trial_matrix: Sequence[Sequence[bool]], k: int = 4) -> float:
    """Reliability metric Pass^k (e.g., Pass^4): mean p_i^k across tasks."""
    if not trial_matrix:
        return 0.0
    vals = []
    for row in trial_matrix:
        if not row:
            continue
        p = sum(1 for x in row if x) / len(row)
        vals.append(p ** k)
    return sum(vals) / len(vals) if vals else 0.0


def flip_rate(baseline_outcomes: Sequence[bool], candidate_outcomes: Sequence[bool]) -> float:
    """Fraction of tasks whose pass/fail outcome flipped between baseline and candidate."""
    if len(baseline_outcomes) != len(candidate_outcomes) or not baseline_outcomes:
        raise ValueError("baseline_outcomes and candidate_outcomes must be non-empty and equal length")
    flips = sum(1 for a, b in zip(baseline_outcomes, candidate_outcomes) if a != b)
    return flips / len(baseline_outcomes)


def cost_per_verified_outcome(total_cost_usd: float, verified_outcomes: int) -> float:
    """CoP: total USD spent divided by machine-verified successful outcomes."""
    if verified_outcomes <= 0:
        return float("inf")
    return total_cost_usd / verified_outcomes


def harness_constraint_effect(constrained_pass_rate: float, unconstrained_pass_rate: float) -> float:
    """Harness Constraint Effect: net pass-rate delta attributable to harness constraints."""
    return round(constrained_pass_rate - unconstrained_pass_rate, 6)


def defect_escape_rate(escaped_defects: int, total_merged_changes: int) -> float:
    """DORA Defect Escape Rate: fraction of merged changes that escaped verification with a defect."""
    if total_merged_changes <= 0:
        return 0.0
    return escaped_defects / total_merged_changes


def false_completion_rate(claimed_done_but_failed: int, total_claimed_done: int) -> float:
    """False Completion Rate: fraction of agent 'done' claims that failed ground-truth verification."""
    if total_claimed_done <= 0:
        return 0.0
    return claimed_done_but_failed / total_claimed_done


def mad_rsi_metrics(
    trial_matrix: Sequence[Sequence[bool]],
    m: int = 4,
    unique_strategy_ratios: Sequence[float] | None = None,
) -> Dict[str, float]:
    """
    MAD-RSI metrics:
    - `Pass^M`: probability of M consecutive passes (reliability floor)
    - `D_q`: quality-weighted diversity = Pass@1 * mean(unique_strategy_ratios)
    """
    p_at_1 = pass_at_1(trial_matrix)
    p_pow_m = pass_pow_k(trial_matrix, k=m)
    div = (
        sum(unique_strategy_ratios) / len(unique_strategy_ratios)
        if unique_strategy_ratios
        else 1.0
    )
    d_q = p_at_1 * max(0.0, min(1.0, div))
    return {
        "pass_at_1": round(p_at_1, 6),
        "pass_pow_m": round(p_pow_m, 6),
        "d_q": round(d_q, 6),
    }