# Template Native Rust Module

This is a copy-and-start native-window module starter.

It lives in `chattycog_gui/module_templates/`, so it will **not** auto-appear in ChattyCog until you copy it into `chattycog_gui/modules/`.

## How to use it

1. Copy this folder into `chattycog_gui/modules/`
2. Rename the copied folder
3. Update:
   - `manifest.json`
   - `visual_load.json`
   - `HANDSHAKE.md`
   - `Cargo.toml`
   - `src/main.rs`
4. Build it once:

```bash
cargo build
```

5. In ChattyCog, use **Modules -> Rescan modules**

## What this starter demonstrates

- real native Rust desktop window via `eframe`
- standalone local state saved in the module folder
- optional bridge handoff through `CHATTYCOG_BRIDGE_STATUS`
- clean behavior inside ChattyCog or outside it

## First things to rename

- package/binary name in `Cargo.toml`
- `module_id` in `manifest.json`
- hosted window title in `visual_load.json`
- `MODULE_ID` constant in `src/main.rs`
