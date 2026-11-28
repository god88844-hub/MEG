---
name: pack-daily
scope: specific
description: Obsidian/Markdown Second Brain (notes, people, projects, inbox, briefs, decisions.md), Karpathy 6-Pass Wiki Compiler, spaced resurfacing (1/3/7/21/60d), Alook team rooms & Nimble web snapshot diffing.
supports:
  - second-brain
  - personal-knowledge
  - daily-briefing
  - team-rooms
differs:
  - pack-vibe
excludes:
  - kernel
  - cad
---
# Pack 6: Daily Usage, Second Brain & Team Rooms

1. Maintain flat-file Markdown vaults (`notes/`, `people/`, `projects/`, `inbox/`, `briefs/`, `decisions.md`).
2. Compile linked notes via the Karpathy 6-Pass Wiki Compiler.
3. Support `/remember`, `/recall`, `/resurface` (`1/3/7/21/60d`), `/review`, `/morning-brief`, `/weekly-review` strictly in `dontAsk` draft-only mode.
4. Separate Alook-style local team rooms into `#intel` (signal/research) vs `#alerts` (urgent action), track Nimble-style dated web snapshot diffs, and normalize custom voice keyterms.