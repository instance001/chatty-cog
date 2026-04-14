# Template Python Module

This is a copy-and-start Python desktop module starter.

It lives in `chattycog_gui/module_templates/`, so it will **not** auto-appear in ChattyCog until you copy it into `chattycog_gui/modules/`.

## How to use it

1. Copy this folder into `chattycog_gui/modules/`
2. Rename the copied folder
3. Update:
   - `manifest.json`
   - `visual_load.json`
   - `HANDSHAKE.md`
   - `src/main.py`
4. Make sure Python is installed and the `py` launcher works
5. In ChattyCog, use **Modules -> Rescan modules**

## What this starter demonstrates

- real native Python desktop window via `tkinter`
- standalone local state saved in the module folder
- optional bridge handoff through `CHATTYCOG_BRIDGE_STATUS`
- clean behavior inside ChattyCog or outside it

## First things to rename

- `module_id` in `manifest.json`
- hosted window title in `visual_load.json`
- `MODULE_ID` in `src/main.py`
- the labels/summary wording in `src/main.py`
