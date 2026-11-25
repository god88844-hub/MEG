# MEG Custom UI & Model Freedom Guide

MEG decouples its **Ring-0 Rust kernel & verification engine** from the **UI layer**, giving users three levels of freedom:

## 1. Built-In Dual-TUI (`/ui classic` & `/ui pro`)
- Switch anytime inside MEG: `/ui classic` (preserved original UI) or `/ui pro` (Titanium Dark, collapsible ToolCards, right Orchestration/Plan/Jobs rail, CSI 2026 sync output).
- Launch directly in Pro mode: `meg --ui pro`.

## 2. No-Compile Customization (`~/.meg/ui.json` or `.meg/ui.json`)
Create `~/.meg/ui.json` (global) or `<workspace>/.meg/ui.json` (per-project) and run `/ui reload`:

    {
      "ui_mode": "pro",
      "show_sidebar": true,
      "compact_tool_cards": true,
      "sync_csi_2026": true,
      "roles": {
        "planner": "gemini-3.1-pro",
        "builder": "qwen2.5-coder:14b",
        "reviewer": "claude-opus-4-6",
        "researcher": "deepseek-chat"
      },
      "colors": {
        "accent": "#38BDF8",
        "viewport_bg": "#0F131C"
      }
    }

## 3. Bring-Your-Own TUI / GUI (`meg ui-bridge`)
Run `meg ui-bridge [-C <workspace>]` to speak **NDJSON (`meg-ui-bridge/v1`)** over stdin/stdout from any language (Python Textual, Go Bubbletea, JS/TS Ink, Rust, or Web):
- **Commands (stdin):** `status`, `set_mode`, `set_roles`, `run`, `quit`
- **Events (stdout):** `ready`, `status`, `mode_updated`, `roles_updated`, `stream_start`, `thinking`, `chunk`, `tool_start`, `tool_end`, `usage`, `done`, `error`, `bye`
- See `examples/custom_tui_starter.py` for a working zero-dependency client.