"""P7-05: Bitter-Lesson Model-Upgrade Decay Sweep (meg-eval/ablate.py):
On new model checkpoints, runs 3-trial single-component ablations and opens
an unmerged PR proposal to delete obsolete scaffolding while preserving
[INSURANCE] safety invariants.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, List, Literal, Sequence

REQUIRED_ABLATION_TRIALS = 3


@dataclass(frozen=True)
class AblationComponent:
    name: str
    file_path: str
    category: Literal["OPTIMIZATION", "INSURANCE"]
    baseline_trials: Tuple[float, float, float]  # 3 trials WITH component
    ablated_trials: Tuple[float, float, float]   # 3 trials WITHOUT component
    token_overhead_per_turn: int


@dataclass(frozen=True)
class AblationDecision:
    component: str
    file_path: str
    action: Literal["delete_obsolete", "keep_useful", "keep_insurance"]
    baseline_mean: float
    ablated_mean: float
    delta: float
    tokens_saved_per_turn: int
    reason: str


@dataclass(frozen=True)
class DecaySweepPrProposal:
    checkpoint: str
    branch: str
    title: str
    components_to_delete: List[str]
    files_to_prune: List[str]
    estimated_tokens_saved_per_turn: int
    merged: Literal[False]
    requires_human_signoff: Literal[True]


def run_bitter_lesson_decay_sweep(
    model_checkpoint: str,
    components: Sequence[AblationComponent],
    *,
    regression_tolerance: float = 0.005,
) -> Dict[str, object]:
    """
    Evaluate 3-trial single-component ablations for a new model checkpoint:
    - `[INSURANCE]` components (safety/verification invariants) are NEVER deleted.
    - `[OPTIMIZATION]` scaffolding whose removal causes <= `regression_tolerance` drop
      (i.e. obsolete on the stronger checkpoint) is queued for a deletion PR.
    """
    decisions: List[AblationDecision] = []
    to_delete: List[AblationDecision] = []

    for comp in components:
        if len(comp.baseline_trials) != REQUIRED_ABLATION_TRIALS or len(comp.ablated_trials) != REQUIRED_ABLATION_TRIALS:
            raise ValueError(
                f"Component '{comp.name}' requires exactly {REQUIRED_ABLATION_TRIALS} baseline and {REQUIRED_ABLATION_TRIALS} ablated trials"
            )
        b_mean = sum(comp.baseline_trials) / REQUIRED_ABLATION_TRIALS
        a_mean = sum(comp.ablated_trials) / REQUIRED_ABLATION_TRIALS
        delta = round(a_mean - b_mean, 6)

        if comp.category == "INSURANCE":
            decisions.append(
                AblationDecision(
                    component=comp.name,
                    file_path=comp.file_path,
                    action="keep_insurance",
                    baseline_mean=round(b_mean, 6),
                    ablated_mean=round(a_mean, 6),
                    delta=delta,
                    tokens_saved_per_turn=0,
                    reason="Protected [INSURANCE] invariant; never removed by decay sweep",
                )
            )
            continue

        if delta >= -regression_tolerance:
            dec = AblationDecision(
                component=comp.name,
                file_path=comp.file_path,
                action="delete_obsolete",
                baseline_mean=round(b_mean, 6),
                ablated_mean=round(a_mean, 6),
                delta=delta,
                tokens_saved_per_turn=comp.token_overhead_per_turn,
                reason=f"Obsolete on {model_checkpoint}: ablating changed score by {delta:+.4f} and saves {comp.token_overhead_per_turn} tok/turn",
            )
            decisions.append(dec)
            to_delete.append(dec)
        else:
            decisions.append(
                AblationDecision(
                    component=comp.name,
                    file_path=comp.file_path,
                    action="keep_useful",
                    baseline_mean=round(b_mean, 6),
                    ablated_mean=round(a_mean, 6),
                    delta=delta,
                    tokens_saved_per_turn=0,
                    reason=f"Still load-bearing on {model_checkpoint}: ablating dropped score by {delta:+.4f}",
                )
            )

    safe_ckpt = "".join(c if c.isalnum() or c in "-_" else "-" for c in model_checkpoint.lower())
    pr = (
        DecaySweepPrProposal(
            checkpoint=model_checkpoint,
            branch=f"decay-sweep/{safe_ckpt}",
            title=f"[decay-sweep] Prune {len(to_delete)} obsolete scaffolding component(s) for {model_checkpoint}",
            components_to_delete=[d.component for d in to_delete],
            files_to_prune=[d.file_path for d in to_delete],
            estimated_tokens_saved_per_turn=sum(d.tokens_saved_per_turn for d in to_delete),
            merged=False,
            requires_human_signoff=True,
        )
        if to_delete
        else None
    )

    return {
        "checkpoint": model_checkpoint,
        "decisions": decisions,
        "obsolete_count": len(to_delete),
        "pr": pr,
    }
from rsi_guards import attribute_harness_delta  # noqa: E402,F401
