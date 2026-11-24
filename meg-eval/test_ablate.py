from ablate import AblationComponent, run_bitter_lesson_decay_sweep


def test_bitter_lesson_3_trial_ablation_prunes_obsolete_scaffolding_and_keeps_insurance():
    res = run_bitter_lesson_decay_sweep(
        "frontier-vNext-2026",
        [
            AblationComponent(
                name="legacy_json_wrapper_nudge",
                file_path="meg-plane/src/legacy_nudge.ts",
                category="OPTIMIZATION",
                baseline_trials=(0.88, 0.89, 0.87),
                ablated_trials=(0.89, 0.89, 0.88),  # removing it even improves score!
                token_overhead_per_turn=320,
            ),
            AblationComponent(
                name="hashline_edit_guard",
                file_path="meg-rs/crates/meg-tools/src/edit.rs",
                category="OPTIMIZATION",
                baseline_trials=(0.90, 0.91, 0.89),
                ablated_trials=(0.75, 0.76, 0.74),  # still load-bearing
                token_overhead_per_turn=140,
            ),
            AblationComponent(
                name="secret_airlock",
                file_path="meg-rs/crates/meg-tools/src/security_guards.rs",
                category="INSURANCE",
                baseline_trials=(0.90, 0.90, 0.90),
                ablated_trials=(0.90, 0.90, 0.90),  # INSURANCE must NEVER be deleted
                token_overhead_per_turn=50,
            ),
        ],
    )
    assert res["obsolete_count"] == 1
    by_name = {d.component: d for d in res["decisions"]}
    assert by_name["legacy_json_wrapper_nudge"].action == "delete_obsolete"
    assert by_name["hashline_edit_guard"].action == "keep_useful"
    assert by_name["secret_airlock"].action == "keep_insurance"

    pr = res["pr"]
    assert pr is not None
    assert pr.components_to_delete == ["legacy_json_wrapper_nudge"]
    assert pr.estimated_tokens_saved_per_turn == 320
    assert pr.merged is False
    assert pr.requires_human_signoff is True