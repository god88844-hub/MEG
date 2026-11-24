"""P6-05 Pack 4 Oracle: Parametric SVG Skeleton, BrickBench DSG, 3DCodeVerse (17 bundles / 28 rules),
18 VisualBench deliverable checklists, and Deterministic 60-FPS Motion Studio."""

from __future__ import annotations

import math
from dataclasses import dataclass
from typing import Dict, Iterable, List, Sequence, Tuple

# ---------------------------------------------------------------------------
# 1. 2-Stage Parametric Skeleton & BrickBench DSG
# ---------------------------------------------------------------------------

def validate_parametric_skeleton(spec: dict) -> Tuple[bool, List[str]]:
    """Enforce Stage 1 (named joints/anchors) before Stage 2 (raw coordinates)."""
    errors: List[str] = []
    anchors = spec.get("anchors")
    primitives = spec.get("primitives")
    if not isinstance(anchors, dict) or len(anchors) == 0:
        errors.append("Stage 1 missing: declare named 'anchors' before raw coordinates")
    if not isinstance(primitives, list) or len(primitives) == 0:
        errors.append("Stage 2 missing: no geometry 'primitives' declared")
    else:
        anchor_names = set(anchors.keys()) if isinstance(anchors, dict) else set()
        for idx, prim in enumerate(primitives):
            ref = prim.get("anchor")
            if not ref or ref not in anchor_names:
                errors.append(f"Primitive #{idx} uses raw coordinates without a declared named anchor")
    return (len(errors) == 0, errors)


@dataclass(frozen=True)
class DsgQuestion:
    qid: str
    parent_id: str | None
    question: str
    passed: bool


def evaluate_brickbench_dsg(questions: Sequence[DsgQuestion]) -> Dict[str, object]:
    """Evaluate BrickBench Davidsonian Scene Graph (DSG): child checks fail if parent fails."""
    by_id = {q.qid: q for q in questions}
    effective: Dict[str, bool] = {}

    def resolve(qid: str) -> bool:
        if qid in effective:
            return effective[qid]
        q = by_id[qid]
        if not q.passed:
            effective[qid] = False
        elif q.parent_id is None:
            effective[qid] = True
        else:
            effective[qid] = resolve(q.parent_id)
        return effective[qid]

    for q in questions:
        resolve(q.qid)
    passed_count = sum(1 for v in effective.values() if v)
    total = len(questions)
    return {
        "score": (passed_count / total) if total else 0.0,
        "passed": passed_count,
        "total": total,
        "failed_ids": [qid for qid, ok in effective.items() if not ok],
    }


# ---------------------------------------------------------------------------
# 2. 3DCodeVerse 17-Bundle Skill Library, 28-Rule Routing Table & 18 VisualBench Checklists
# ---------------------------------------------------------------------------

THREED_CODEVERSE_17_BUNDLES: Tuple[str, ...] = (
    "bpy_modeling",
    "bpy_materials",
    "bpy_lighting",
    "bpy_rigging",
    "bpy_animation",
    "cadquery_solid",
    "cadquery_assembly",
    "cadquery_step_export",
    "openscad_parametric",
    "openscad_csg",
    "threejs_scene",
    "threejs_pbr",
    "threejs_postfx",
    "urdf_kinematics",
    "urdf_collision",
    "glsl_fragment",
    "opengl_pipeline",
)

# 28 deterministic keyword -> (engine, bundle) routing rules
THREED_CODEVERSE_28_RULES: Tuple[Tuple[str, str, str], ...] = (
    ("blender", "bpy", "bpy_modeling"),
    ("subsurf", "bpy", "bpy_modeling"),
    ("principled_bsdf", "bpy", "bpy_materials"),
    ("cycles", "bpy", "bpy_lighting"),
    ("hdri", "bpy", "bpy_lighting"),
    ("armature", "bpy", "bpy_rigging"),
    ("keyframe_insert", "bpy", "bpy_animation"),
    ("cadquery", "CadQuery", "cadquery_solid"),
    ("workplane", "CadQuery", "cadquery_solid"),
    ("fillet", "CadQuery", "cadquery_solid"),
    ("chamfer", "CadQuery", "cadquery_solid"),
    ("assembly", "CadQuery", "cadquery_assembly"),
    ("step_export", "CadQuery", "cadquery_step_export"),
    ("openscad", "OpenSCAD", "openscad_parametric"),
    ("difference", "OpenSCAD", "openscad_csg"),
    ("minkowski", "OpenSCAD", "openscad_csg"),
    ("three.js", "Three.js", "threejs_scene"),
    ("orbitcontrols", "Three.js", "threejs_scene"),
    ("meshstandardmaterial", "Three.js", "threejs_pbr"),
    ("effectcomposer", "Three.js", "threejs_postfx"),
    ("urdf", "URDF", "urdf_kinematics"),
    ("revolute", "URDF", "urdf_kinematics"),
    ("inertial", "URDF", "urdf_collision"),
    ("glsl", "GLSL", "glsl_fragment"),
    ("raymarch", "GLSL", "glsl_fragment"),
    ("sdf", "GLSL", "glsl_fragment"),
    ("opengl", "OpenGL", "opengl_pipeline"),
    ("vao", "OpenGL", "opengl_pipeline"),
)

VISUALBENCH_18_CHECKLISTS: Tuple[str, ...] = (
    "01_svg_parametric_anchors",
    "02_svg_viewbox_clipping",
    "03_svg_typography_contrast",
    "04_diagram_arrow_endpoints",
    "05_chart_axis_labels",
    "06_chart_data_fidelity",
    "07_cad_watertight_manifold",
    "08_cad_dimensional_tolerance",
    "09_cad_no_interpenetration",
    "10_cad_connected_components",
    "11_3d_camera_framing",
    "12_3d_lighting_no_blackout",
    "13_3d_material_pbr_sanity",
    "14_shader_zero_compile_error",
    "15_motion_60fps_deterministic_seek",
    "16_motion_audio_beat_alignment",
    "17_motion_chroma_spill_clean",
    "18_motion_contact_strip_phone_sheets",
)


def route_3d_codeverse(prompt: str) -> Dict[str, str]:
    low = prompt.lower()
    for keyword, engine, bundle in THREED_CODEVERSE_28_RULES:
        if keyword in low:
            return {"engine": engine, "bundle": bundle, "matched_rule": keyword}
    return {"engine": "Three.js", "bundle": "threejs_scene", "matched_rule": "default"}


@dataclass(frozen=True)
class AabbBox:
    name: str
    min_xyz: Tuple[float, float, float]
    max_xyz: Tuple[float, float, float]


def check_spatial_collision_and_connectivity(
    boxes: Sequence[AabbBox],
    tolerance: float = 1e-3,
) -> Dict[str, object]:
    """Detect strict interior collisions and disconnected floating parts."""
    collisions: List[str] = []
    adj: Dict[int, set[int]] = {i: set() for i in range(len(boxes))}

    for i in range(len(boxes)):
        a = boxes[i]
        for j in range(i + 1, len(boxes)):
            b = boxes[j]
            overlaps = [
                min(a.max_xyz[k], b.max_xyz[k]) - max(a.min_xyz[k], b.min_xyz[k])
                for k in range(3)
            ]
            # Strict interior overlap on all 3 axes beyond tolerance = collision.
            if all(ov > tolerance for ov in overlaps):
                collisions.append(f"{a.name}<->{b.name}")
            # Touching or overlapping within tolerance = connected.
            if all(ov >= -tolerance for ov in overlaps):
                adj[i].add(j)
                adj[j].add(i)

    # BFS connectivity check
    visited = set()
    if boxes:
        stack = [0]
        while stack:
            cur = stack.pop()
            if cur in visited:
                continue
            visited.add(cur)
            stack.extend(adj[cur] - visited)
    connected = len(visited) == len(boxes)
    return {"collisions": collisions, "connected": connected, "ok": len(collisions) == 0 and connected}


# ---------------------------------------------------------------------------
# 3. Deterministic 60-FPS Motion Studio (window.seek(t), springs, beats, chroma, sheets)
# ---------------------------------------------------------------------------

def track_spring(
    t: float,
    keys: Sequence[Tuple[float, float]],
    stiffness: float = 180.0,
    damping: float = 24.0,
) -> float:
    """Closed-form multi-target spring `track(t, keys)` -- pure function of time `t` for `window.seek(t)`."""
    if not keys:
        return 0.0
    ordered = sorted(keys, key=lambda kv: kv[0])
    if t <= ordered[0][0]:
        return float(ordered[0][1])

    val = float(ordered[0][1])
    omega0 = math.sqrt(max(1e-6, stiffness))
    zeta = damping / (2.0 * omega0)

    for idx in range(1, len(ordered)):
        t_key, target = ordered[idx]
        if t < t_key:
            break
        dt = t - t_key
        prev_target = ordered[idx - 1][1]
        delta = target - prev_target
        if zeta >= 1.0:
            # Critically / overdamped closed-form step response
            envelope = math.exp(-omega0 * dt) * (1.0 + omega0 * dt)
        else:
            omega_d = omega0 * math.sqrt(1.0 - zeta * zeta)
            envelope = math.exp(-zeta * omega0 * dt) * (
                math.cos(omega_d * dt) + (zeta / math.sqrt(1.0 - zeta * zeta)) * math.sin(omega_d * dt)
            )
        val += delta * (1.0 - envelope)
    return val


def align_beats_to_frames(beat_times_sec: Sequence[float], fps: int = 60) -> List[int]:
    """Map librosa-style beat timestamps to deterministic integer 60-FPS frame indices."""
    return [int(round(t * fps)) for t in beat_times_sec if t >= 0.0]


def chroma_cutout_mask(
    rgb_pixels: Sequence[Tuple[int, int, int]],
    key_hex: str = "#00B140",
    tolerance: float = 55.0,
) -> List[bool]:
    """Classify foreground (True) vs `#00B140` chroma background (False) pixels."""
    clean = key_hex.lstrip("#")
    kr, kg, kb = int(clean[0:2], 16), int(clean[2:4], 16), int(clean[4:6], 16)
    mask: List[bool] = []
    for r, g, b in rgb_pixels:
        dist = math.sqrt((r - kr) ** 2 + (g - kg) ** 2 + (b - kb) ** 2)
        mask.append(dist > tolerance)
    return mask


MOTION_STUDIO_SHEETS: Tuple[str, str, str] = ("contact.png", "strip.png", "phone.png")


def build_ffmpeg_sheet_commands(video_path: str, out_dir: str) -> Dict[str, List[str]]:
    return {
        "contact.png": ["ffmpeg", "-y", "-i", video_path, "-vf", "fps=1,scale=320:-1,tile=4x4", f"{out_dir}/contact.png"],
        "strip.png": ["ffmpeg", "-y", "-i", video_path, "-vf", "fps=0.5,scale=240:-1,tile=8x1", f"{out_dir}/strip.png"],
        "phone.png": ["ffmpeg", "-y", "-i", video_path, "-vf", "fps=1,scale=180:320,tile=3x2", f"{out_dir}/phone.png"],
    }


def verify_motion_frame_diff(
    frame_at_2000ms: Sequence[int],
    frame_at_15000ms: Sequence[int],
    min_mean_abs_diff: float = 1.0,
) -> Dict[str, object]:
    """Verify video is actually animating between 2000ms and 15000ms (catches frozen/static renders)."""
    if len(frame_at_2000ms) != len(frame_at_15000ms) or len(frame_at_2000ms) == 0:
        return {"ok": False, "mean_abs_diff": 0.0, "reason": "mismatched or empty frame buffers"}
    mad = sum(abs(a - b) for a, b in zip(frame_at_2000ms, frame_at_15000ms)) / len(frame_at_2000ms)
    return {"ok": mad >= min_mean_abs_diff, "mean_abs_diff": mad}