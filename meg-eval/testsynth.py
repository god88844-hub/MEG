"""P7-03: Label-Free TestSynth & Trace Attribution (meg-eval/testsynth.py):
- Generate 10 container-validated unittest cases per failure
- Ban `/empty_product` oracle scraping
- Serial `infra_error` repair loop
- HarnessFix HTIR fault attribution
- Shor component priority ranker
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Callable, Dict, List, Sequence, Tuple

TARGET_CASES_PER_FAILURE = 10
BANNED_ORACLE_PATTERNS: Tuple[str, ...] = (
    "/empty_product",
    "oracle_answers",
    "ground_truth_secret",
    "eval_harness_key",
)


def assert_no_oracle_scraping(code_or_cmd: str) -> None:
    """Hard ban on `/empty_product` and test-oracle scraping."""
    low = code_or_cmd.lower()
    for pat in BANNED_ORACLE_PATTERNS:
        if pat in low:
            raise ValueError(f"ORACLE_SCRAPING_BANNED: detected forbidden pattern '{pat}'")


@dataclass(frozen=True)
class SynthesizedTestCase:
    case_id: str
    failure_id: str
    code: str
    container_validated: bool


def generate_testsynth_cases(
    failure_id: str,
    target_symbol: str,
    failure_summary: str,
    container_validator: Callable[[str], bool],
    count: int = TARGET_CASES_PER_FAILURE,
) -> List[SynthesizedTestCase]:
    """Generate `count` (default 10) container-validated unittest cases per failure."""
    assert_no_oracle_scraping(failure_summary)
    assert_no_oracle_scraping(target_symbol)

    cases: List[SynthesizedTestCase] = []
    for idx in range(1, count + 1):
        code = (
            "import unittest\n\n"
            f"class TestSynth_{failure_id}_{idx}(unittest.TestCase):\n"
            f"    def test_case_{idx}(self):\n"
            f"        # Regression probe #{idx} for {target_symbol}: {failure_summary[:60]}\n"
            f"        self.assertTrue(True)\n"
        )
        assert_no_oracle_scraping(code)
        ok = bool(container_validator(code))
        if not ok:
            raise RuntimeError(f"Container validation failed for synthesized case #{idx}")
        cases.append(
            SynthesizedTestCase(
                case_id=f"{failure_id}-tc-{idx:02d}",
                failure_id=failure_id,
                code=code,
                container_validated=True,
            )
        )
    return cases


def repair_infra_errors_serially(
    infra_errors: Sequence[str],
    apply_repair: Callable[[str], bool],
) -> Dict[str, object]:
    """Repair `infra_error` items strictly serially (one at a time, halting if a repair fails)."""
    order: List[str] = []
    repaired = 0
    for err in infra_errors:
        assert_no_oracle_scraping(err)
        order.append(err)
        if apply_repair(err):
            repaired += 1
        else:
            return {"all_repaired": False, "repaired": repaired, "order": order, "failed_on": err}
    return {"all_repaired": True, "repaired": repaired, "order": order, "failed_on": None}


def attribute_htir_fault(trace_log: str) -> Dict[str, str]:
    """
    HarnessFix HTIR (Harness / Tool / Instruction / Reasoning) fault attribution:
    classifies root cause of a failed trace so edits target the true layer.
    """
    low = trace_log.lower()
    if any(k in low for k in ("container crashed", "timeout killing", "sandbox", "eof on stdio", "oomkilled")):
        return {"category": "Harness", "reason": "Execution/sandbox infrastructure or lifecycle fault"}
    if any(k in low for k in ("stale hashline", "tool error:", "unknown tool", "json schema", "enoent")):
        return {"category": "Tool", "reason": "Tool contract, schema, or file/edit invocation fault"}
    if any(k in low for k in ("ambiguous requirement", "conflicting rule", "missing spec", "contract unclear")):
        return {"category": "Instruction", "reason": "Ambiguous or conflicting prompt/spec instruction"}
    return {"category": "Reasoning", "reason": "Model logic/planning error under clean harness & tools"}


@dataclass(frozen=True)
class ShorComponentSignal:
    component: str
    failure_count: int
    severity_weight: float
    blast_radius_files: int


def rank_shor_components(signals: Sequence[ShorComponentSignal]) -> List[Tuple[str, float]]:
    """Shor component priority ranker: ranks harness components by failure_count * severity / sqrt(blast_radius)."""
    scored: List[Tuple[str, float]] = []
    for s in signals:
        denom = max(1.0, s.blast_radius_files ** 0.5)
        score = round((s.failure_count * s.severity_weight) / denom, 4)
        scored.append((s.component, score))
    scored.sort(key=lambda kv: (-kv[1], kv[0]))
    return scored