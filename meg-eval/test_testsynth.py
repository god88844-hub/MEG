import pytest

from testsynth import (
    ShorComponentSignal,
    assert_no_oracle_scraping,
    attribute_htir_fault,
    generate_testsynth_cases,
    rank_shor_components,
    repair_infra_errors_serially,
)


def test_testsynth_generates_10_container_validated_cases_and_bans_empty_product():
    cases = generate_testsynth_cases(
        failure_id="F42",
        target_symbol="parse_json",
        failure_summary="handles trailing comma cleanly",
        container_validator=lambda code: "unittest.TestCase" in code,
    )
    assert len(cases) == 10
    assert all(c.container_validated for c in cases)

    with pytest.raises(ValueError, match="ORACLE_SCRAPING_BANNED"):
        assert_no_oracle_scraping("cat /empty_product/solution.txt")


def test_serial_infra_repair_htir_attribution_and_shor_ranker():
    log = []
    res = repair_infra_errors_serially(
        ["infra_error: missing wheel", "infra_error: stale socket"],
        lambda e: (log.append(e) is None),
    )
    assert res["all_repaired"] is True
    assert res["repaired"] == 2
    assert log == ["infra_error: missing wheel", "infra_error: stale socket"]

    assert attribute_htir_fault("Container crashed: OOMKilled")["category"] == "Harness"
    assert attribute_htir_fault("tool error: stale hashline anchor")["category"] == "Tool"
    assert attribute_htir_fault("conflicting rule in contract")["category"] == "Instruction"
    assert attribute_htir_fault("off-by-one arithmetic error")["category"] == "Reasoning"

    ranked = rank_shor_components(
        [
            ShorComponentSignal("edit_hashline", failure_count=10, severity_weight=2.0, blast_radius_files=1),
            ShorComponentSignal("context_trim", failure_count=4, severity_weight=1.0, blast_radius_files=4),
        ]
    )
    assert ranked[0][0] == "edit_hashline"
    assert ranked[0][1] > ranked[1][1]