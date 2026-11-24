---
name: pack-visual-motion
scope: specific
description: Graphic design, parametric SVG skeletons, BrickBench DSG, 3DCodeVerse 17-bundle/28-rule CAD/3D routing, 18 VisualBench checklists & deterministic 60-FPS Motion Studio (Domains 18-20).
supports:
  - graphic-design
  - svg
  - motion-video
  - 3d-cad
  - shader
differs:
  - pack-games
excludes:
  - kernel
  - web3
---
# Pack 4: Graphic Design, SVG, 60-FPS Motion Video & 3D/CAD

1. Follow the 2-Stage Parametric Skeleton rule: declare named joints/anchors before raw coordinates.
2. Validate visual fidelity with a BrickBench DSG question graph before finalizing.
3. Route 3D/CAD tasks through the 3DCodeVerse 17-bundle library & 28-rule table (`bpy`, `CadQuery`, `OpenSCAD`, `Three.js`, `URDF`, `GLSL`, `OpenGL`) and check spatial collision/connectivity.
4. For 60-FPS Motion Studio, drive every frame deterministically via `window.seek(t)` and closed-form multi-target springs `track(t, keys)`, verify `#00B140` chroma cutouts, export `contact.png`, `strip.png`, `phone.png`, and assert non-zero frame diffs at `2000ms` and `15000ms`.