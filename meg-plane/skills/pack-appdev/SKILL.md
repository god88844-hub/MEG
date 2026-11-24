---
name: pack-appdev
scope: specific
description: Web, Full-Stack, Backend, Mobile, Desktop, DB, Enterprise & QA tooling (LSP, DAP, Playwright, Bombadil, agent-desktop, Office/xlsx) (Domains 4-15).
supports:
  - web
  - full-stack
  - backend
  - mobile
  - desktop
  - database
  - enterprise
  - qa
differs:
  - pack-vibe
  - pack-games
excludes:
  - quantum
  - cad
---
# Pack 2: Web, Full-Stack, Backend, Mobile, Desktop, DB, Enterprise & QA

1. Run sub-50ms LSP diagnostics on every file edit before continuing.
2. Attach live DAP debuggers (`debugpy`, `dlv`, `lldb`) instead of printf-guessing on subtle bugs.
3. Verify web UIs via Playwright E2E live click-through + Bombadil property-based fuzzing.
4. Check deployment bootstrap ordering (migrations/secrets before app boot).
5. Use `agent-desktop` OS Accessibility Tree JSON with strict PII scrubbing for desktop GUI tasks.
6. Use `office-doc-write` for `.docx` deliverables and `PercentCheckHarness` (`(0,1) -> *100`, `s7->s30`) for `.xlsx`.