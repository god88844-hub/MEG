import math

from metrics import (
    cost_per_verified_outcome,
    defect_escape_rate,
    false_completion_rate,
    flip_rate,
    harness_constraint_effect,
    mad_rsi_metrics,
    pass_at_1,
    pass_pow_k,
    unbiased_pass_at_k,
)


def test_pass_at_k_pass_pow_4_flip_and_cop():
    assert math.isclose(unbiased_pass_at_k(4, 2, 1), 0.5)
    assert math.isclose(unbiased_pass_at_k(4, 2, 2), 1.0 - (1 / 6))
    assert unbiased_pass_at_k(4, 4, 4) == 1.0

    trials = [
        [True, True, True, True],   # p = 1.0 -> p^4 = 1.0
        [True, False, True, False], # p = 0.5 -> p^4 = 0.0625
    ]
    assert math.isclose(pass_at_1(trials), 0.75)
    assert math.isclose(pass_pow_k(trials, 4), 0.53125)
    assert math.isclose(flip_rate([True, False, True, False], [True, True, True, False]), 0.25)
    assert math.isclose(cost_per_verified_outcome(12.5, 5), 2.5)
    assert math.isinf(cost_per_verified_outcome(5.0, 0))


def test_dora_and_mad_rsi_metrics():
    assert math.isclose(harness_constraint_effect(0.88, 0.72), 0.16)
    assert math.isclose(defect_escape_rate(2, 50), 0.04)
    assert math.isclose(false_completion_rate(1, 20), 0.05)

    m = mad_rsi_metrics([[True, True, True, True]], m=4, unique_strategy_ratios=[0.8])
    assert m["pass_at_1"] == 1.0
    assert m["pass_pow_m"] == 1.0
    assert math.isclose(m["d_q"], 0.8)