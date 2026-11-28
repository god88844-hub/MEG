---
name: pack-vibe
scope: specific
description: Vibe-to-Spec PRD pipeline, 3 Goldfish critics, AskUserQuestion forms, state-machine refactor & /taste anti-slop UI checks (Domains 1-3).
supports:
  - vibe-coding
  - product-management
  - scrum
  - ui-ux-wireframing
differs:
  - pack-appdev
excludes:
  - kernel
  - quantum
---
# Pack 1: Vibe Coders, PM, Scrum & UI/UX Wireframing

1. Turn vague prompts into an Elephant `PRD.md` before coding.
2. Run 3 stateless Goldfish critics (**Contrarian**, **Edge-Case**, **Token Economist**) before slicing.
3. Slice into 1-feature fresh-context tasks with explicit acceptance criteria.
4. Use structured `AskUserQuestion` forms (<=2 high-impact MCQs) when requirements are ambiguous.
5. Run `/refactor-state-machines` across UX, DX, and AX flows.
6. Run `/taste` anti-slop checks (`a11y`, `smell.md`, `typeset.md`, 1440px desktop & 360px mobile viewports).