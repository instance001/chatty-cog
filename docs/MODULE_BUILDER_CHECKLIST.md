# Module Builder Checklist

Use this as a quick preflight when turning any standalone tool into a ChattyCog-compatible module.

## 1) Pick a starter

- Read `docs/MODULE_TEMPLATE_CHOOSER.md`
- Copy one starter into `chattycog_gui/modules/`
  - `template_module/` for webview
  - `template_native_rust_module/` for native Rust
  - `template_python_module/` for native Python

## 2) Rename the basics

- rename the copied folder
- update `manifest.json`
  - `module_id`
  - `display_name`
  - `description`
- update the module title shown by the app/UI
- update any hardcoded `MODULE_ID` constant in code

## 3) Keep the module standalone first

Before worrying about ChattyCog:

- make sure the module runs by itself
- make sure its own UI/state works normally
- make sure it can save/load whatever it needs on its own

Rule of thumb:

- ChattyCog should host the module
- ChattyCog should not become the module's real runtime brain

Design target:

- build the module so it can participate in a larger in-app workflow loop, not just open as an isolated tab
- assume the user may move between the main AI, this module, and sibling tools such as `chatty-art`, `chatty-lora`, or `chatty-quest` without leaving the shell
- keep the host target explicit: ChattyCog modules are not the same thing as Chatty-EDU modules, even if the tool idea overlaps

## 4) Add visual hosting

Create or update `visual_load.json`.

Choose one:

- `webview` if the module is HTML/CSS/JS
- `native_window` if the module opens a real desktop window

Check:

- launch path is correct
- working directory is correct
- window title is stable enough for docking
- optional build command works if the module needs one

## 5) Add the human handshake

Create or update `HANDSHAKE.md`.

Make sure it explains:

- what the module is for
- what inputs it expects
- what outputs it produces
- what a good suspend handoff should contain

## 6) Add the optional bridge

Use:

- `docs/MODULE_BRIDGE.md`
- `docs/MODULE_BRIDGE_SNIPPETS.md`
- `docs/MODULE_LOG_SOURCES_TEMPLATE.json`

Goal:

- module keeps owning its own UI/state
- module optionally reports a short `summary` + `snapshot`
- if the module already has useful logs, it can declare them for ChattyCog to tail
- ChattyCog reads that handoff when the module tab is left or closed
- the next tool or orchestrator pass should be able to pick that handoff up without guesswork

For webviews:

- use `window.chattyCogBridge.updateStatus(...)`

For native apps:

- write to `CHATTYCOG_BRIDGE_STATUS` if it exists

If the module already writes its own logs:

- add `bridge/log_sources.json`
- keep paths module-relative
- let Bookkeeper use the recent log tail for auto-generated handoff context

## 7) Keep the bridge lightweight

Good:

- one short paragraph in `summary`
- richer state in `snapshot`
- a few stable tags
- optional structured `payload`

Avoid:

- treating the bridge as the module's main database
- coupling the module tightly to ChattyCog internals
- assuming the bridge exists when the app runs standalone
- assuming a ChattyCog module should automatically be dropped into Chatty-EDU without a separate intentional adaptation

## 8) Test both modes

Standalone check:

- launch the module by itself
- confirm it still works without ChattyCog

Hosted check:

- open ChattyCog
- use **Modules -> Rescan modules**
- open the module tab
- confirm the real UI appears in the tab
- use the module
- switch away from the tab
- confirm ChattyCog can read what happened

## 9) Confirm portability

Ask:

- if I remove the bridge logic, does the module still run standalone?
- if I remove `visual_load.json`, does the module still remain a valid standalone tool?

If yes, you are keeping the boundary clean.

## 10) Nice finishing touches

Recommended:

- add a zero-knowledge `USER_MANUAL.md` inside the module folder
- read `docs/MODULE_PACKAGING_GUIDE.md` before shipping
- do one pass with `docs/MODULE_REVIEW_RUBRIC.md`
- keep filenames and labels obvious
- keep launch/build paths relative to the module folder when possible
- add a stable window title for native docking
- keep bridge updates meaningful, not noisy

## Ship-ready definition

A module is in good shape when:

- it runs standalone
- it can be hosted inside a ChattyCog tab
- it reports a clean suspend handoff through the optional bridge
- it plays nicely with multi-step loops across ChattyCog and sibling specialist modules
- it stays simple enough that removing the plug cleanly removes ChattyCog compatibility without breaking the tool itself
