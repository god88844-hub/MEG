---
name: pack-games
scope: specific
description: 2D/3D Games M5 7-Subsystem ECS, .hot.* 60-FPS lint, WebGL 0-console-error check, BFS playtester & Kepler 1.0 ARC-AGI-3 world models (Domains 16-17).
supports:
  - 2d-games
  - 3d-games
  - physics
  - arc-agi-3
differs:
  - pack-visual-motion
excludes:
  - web3
  - enterprise
---
# Pack 3: 2D/3D Games, Physics & ARC-AGI-3 World Models

1. Scaffold M5 7-Subsystem ECS (`SCHEMA.md` + Input, Physics, Collision, AI, Audio, Render, HUD).
2. Enforce `.hot.*` zero-allocation and zero-unseeded-RNG 60-FPS lint on hot-loop files.
3. Require WebGL 0-console-error verification and automated BFS playtester reachability.
4. For ARC-AGI-3 world models (`Kepler 1.0` / `Schema`), certify `step(state, action)` with 100% `run_backtest` match and auto-`RESET` on `GAME_OVER`.