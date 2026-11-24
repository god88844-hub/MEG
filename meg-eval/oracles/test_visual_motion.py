from visual_motion import (
    AabbBox,
    DsgQuestion,
    MOTION_STUDIO_SHEETS,
    THREED_CODEVERSE_17_BUNDLES,
    THREED_CODEVERSE_28_RULES,
    VISUALBENCH_18_CHECKLISTS,
    align_beats_to_frames,
    build_ffmpeg_sheet_commands,
    check_spatial_collision_and_connectivity,
    chroma_cutout_mask,
    evaluate_brickbench_dsg,
    route_3d_codeverse,
    track_spring,
    verify_motion_frame_diff,
    validate_parametric_skeleton,
)


def test_parametric_skeleton_and_brickbench_dsg():
    ok, errs = validate_parametric_skeleton(
        {
            "anchors": {"shoulder": (0, 0), "elbow": (10, 0)},
            "primitives": [{"kind": "line", "anchor": "shoulder"}, {"kind": "circle", "anchor": "elbow"}],
        }
    )
    assert ok and errs == []

    bad_ok, bad_errs = validate_parametric_skeleton({"anchors": {}, "primitives": [{"kind": "rect", "x": 10}]})
    assert not bad_ok and len(bad_errs) == 2

    dsg = evaluate_brickbench_dsg(
        [
            DsgQuestion("q1", None, "Is the base plate present?", True),
            DsgQuestion("q2", "q1", "Is the tower attached?", False),
            DsgQuestion("q3", "q2", "Is the flag on top of the tower?", True),  # fails transitively via q2
        ]
    )
    assert dsg["passed"] == 1
    assert set(dsg["failed_ids"]) == {"q2", "q3"}


def test_3dcodeverse_17_bundles_28_rules_18_visualbench_and_spatial_checks():
    assert len(THREED_CODEVERSE_17_BUNDLES) == 17
    assert len(THREED_CODEVERSE_28_RULES) == 28
    assert len(VISUALBENCH_18_CHECKLISTS) == 18

    assert route_3d_codeverse("Build a CadQuery workplane bracket")["engine"] == "CadQuery"
    assert route_3d_codeverse("Write a GLSL raymarch shader")["engine"] == "GLSL"
    assert route_3d_codeverse("Create a URDF revolute joint")["engine"] == "URDF"

    # Touching boxes -> connected & collision-free
    good = check_spatial_collision_and_connectivity(
        [
            AabbBox("base", (0.0, 0.0, 0.0), (10.0, 10.0, 2.0)),
            AabbBox("pillar", (4.0, 4.0, 2.0), (6.0, 6.0, 8.0)),
        ]
    )
    assert good["ok"] is True

    # Interpenetrating boxes -> collision flagged
    bad = check_spatial_collision_and_connectivity(
        [
            AabbBox("a", (0.0, 0.0, 0.0), (5.0, 5.0, 5.0)),
            AabbBox("b", (2.0, 2.0, 2.0), (7.0, 7.0, 7.0)),
        ]
    )
    assert bad["ok"] is False
    assert "a<->b" in bad["collisions"]


def test_deterministic_60fps_motion_studio_springs_beats_chroma_sheets_and_frame_diff():
    keys = [(0.0, 0.0), (1.0, 100.0)]
    # Deterministic pure function of time t
    v_a = track_spring(1.25, keys)
    v_b = track_spring(1.25, keys)
    assert v_a == v_b
    assert track_spring(0.0, keys) == 0.0
    assert abs(track_spring(5.0, keys) - 100.0) < 0.01

    assert align_beats_to_frames([0.0, 0.5, 1.0], fps=60) == [0, 30, 60]
    assert chroma_cutout_mask([(0, 177, 64), (240, 20, 20)]) == [False, True]

    cmds = build_ffmpeg_sheet_commands("out.mp4", "sheets")
    assert tuple(cmds.keys()) == MOTION_STUDIO_SHEETS

    assert verify_motion_frame_diff([10, 20, 30], [10, 20, 30])["ok"] is False
    assert verify_motion_frame_diff([10, 20, 30], [100, 120, 140])["ok"] is True