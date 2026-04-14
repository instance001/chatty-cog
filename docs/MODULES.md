# ChattyCog Modules (Plug-In System)

This document describes how to create drop-in modules for ChattyCog.

## What is a module?

A module is a folder dropped into `chattycog_gui/modules/` that contains a `manifest.json`.

ChattyCog scans that directory at startup (and on **Modules -> Rescan modules**) and lets the user open modules as closable tabs.

Starter templates live in:

- `chattycog_gui/module_templates/`

That location is intentional so templates do not appear as live modules until you copy them into `chattycog_gui/modules/`.

If you are unsure which starter fits your module best:

- `docs/MODULE_TEMPLATE_CHOOSER.md`
- `docs/MODULE_PACKAGING_GUIDE.md`
- `docs/MODULE_REVIEW_RUBRIC.md`
- `docs/MODULE_RELEASE_NOTES_TEMPLATE.md`
- `docs/CHANGELOG_TEMPLATE.md`
- `docs/MODULE_SUBMISSION_TEMPLATE.md`

## Directory layout

```
chattycog_gui/
  modules/
    your_module/
      manifest.json
      visual_load.json      (optional; advertises a standalone UI ChattyCog can host)
      network_capabilities.json (optional; declares intentional LAN-sharing support)
      bridge/status.json    (optional runtime file; module-reported handoff status)
      bridge/incoming_assets/ (optional runtime lane; approved module files/payloads land here)
      bridge/log_sources.json (optional; declares module-local logs for debrief context)
      ui.json               (recommended for in-tab UI)
      HANDSHAKE.md          (recommended)
      STATE_TEMPLATE.md     (optional; used if ui.json is missing)
      ...any other files your module needs...
```

Only the presence of `manifest.json` is required for discovery.

## `manifest.json` schema

Minimum required fields:

```json
{
  "module_id": "research_lab",
  "display_name": "Research Lab",
  "icon": "flask",
  "description": "Notes + experiments + references."
}
```

Required fields:
- `module_id` (string): stable identifier used for logging, search filters, and routing.
- `display_name` (string): what shows up in the Modules menu and tab label.
- `icon` (string): reserved for UI use (currently displayed as text).
- `description` (string): short human description shown in the module tab.

Optional fields (supported today):
- `ai_enabled` (bool): if `true`, ChattyCog shows a "Module AI (demo)" runner in the tab.
- `default_model` (string): a GGUF filename (from `chattycog_gui/models/`) to preselect for the module AI runner.
- `visual_load` (object): optional inline visual-hosting block. In practice, a separate `visual_load.json` companion file is cleaner.
- `network_capabilities` (object): optional inline declaration of which network-sharing lanes the module intentionally supports. In practice, a separate `network_capabilities.json` companion file is cleaner.

Notes:
- `module_id` and `display_name` must be non-empty or the module is ignored.
- Unknown fields are ignored.

## Discovery rules

Discovery scans only:
- `chattycog_gui/modules/*/manifest.json`
- `chattycog_gui/modules/*/visual_load.json` (optional companion file)
- `chattycog_gui/modules/*/network_capabilities.json` (optional companion file)

It does not recurse deeper than one folder level.

If a manifest fails to parse, the module is skipped.

## Opening and closing modules

Modules do not auto-open.

- Open via **Modules -> Open: <display_name>** (creates a closable tab).
- Close via the `x` on the tab.
  - If the module is currently running a task (module AI demo), the close becomes "close pending" and the tab closes when the task finishes.

## Module tab behavior

Every module tab provides:
- manifest display (`display_name`, `description`, `icon`)
- a **Workspace** surface (module-owned UI or template-backed editor)
- a **Suspend rundown** surface ("what happened in this module")

If a module advertises `visual_load.json` with a supported native window target, ChattyCog can launch that standalone app and dock it directly inside the tab. The usual workspace / rundown / module-AI helpers stay available under a collapsed **ChattyCog bridge** panel.

### Three display paths

In practice, modules show up in one of three ways:

1. **Docked native app**
   - Used by modules with a real desktop window.
   - ChattyCog launches that real app and docks it into the tab.

2. **Docked web dashboard**
   - Used by modules whose real UI is HTML/CSS/JS.
   - ChattyCog hosts that real browser-style dashboard in a webview.
   - Even if it looks more like a form or dashboard, this is still the module's own UI, not the fallback.

3. **ChattyCog fallback workspace**
   - Used when the module has no hosted UI.
   - ChattyCog provides the visual shell through `ui.json` or the workspace editor.

Why this split exists:
- desktop apps should keep their real desktop UI
- browser-style tools should keep their real browser UI
- CLI/headless tools still need a usable surface inside ChattyCog

So the "middle ground" UI is only meant for modules without their own hosted surface.

See also:
- `docs/MODULE_BUILDER_CHECKLIST.md`
- `docs/MODULE_PACKAGING_GUIDE.md`
- `docs/MODULE_REVIEW_RUBRIC.md`
- `docs/MODULE_RELEASE_NOTES_TEMPLATE.md`
- `docs/CHANGELOG_TEMPLATE.md`
- `docs/MODULE_SUBMISSION_TEMPLATE.md`
- `docs/MODULE_TEMPLATE_CHOOSER.md`
- `docs/MODULE_VISUAL_LOAD.md`
- `docs/MODULE_BRIDGE.md`
- `docs/MODULE_BRIDGE_SNIPPETS.md`
- `chattycog_gui/module_templates/template_module/`
- `chattycog_gui/module_templates/template_native_rust_module/`
- `chattycog_gui/module_templates/template_python_module/`
- `docs/MODULE_VISUAL_LOAD_TEMPLATE.json`
- `docs/MODULE_VISUAL_LOAD_WEBVIEW_TEMPLATE.json`

### Module Workspace UI (`ui.json`) - recommended

If a module folder contains `ui.json`, ChattyCog renders it as a native-feeling module surface (section cards + sidebar when available) and persists values to:
- `chattycog_gui/modules/<module>/state.json`

This is the recommended way for modules to provide their "own display" inside a ChattyCog tab without custom code changes in ChattyCog itself.

Important:
- `ui.json` is the right fit for modules that do **not** ship their own hosted app or web dashboard
- if a module already has `visual_load.json`, that hosted `native_window` or `webview` surface is the primary UI
- `ui.json` then acts as the fallback / bridge-side workspace, not the main hosted surface

The surface schema now supports reusable blocks such as:
- editable fields
- instruction / status callouts
- compact stats
- key/value record summaries
- filterable checklist, timeline, and kanban views with quick presets
- lightweight dependency-flow graphs
- lightweight table previews
- markdown-style content previews
- progress and simple bar-chart visuals
- nested tabbed panes, split workspaces, and layout view presets
- collapsible accordions, inspector panels, and focused pane lenses
- built-in action button groups
- safe file lists and artifact previews inside the module folder

See also:
- `docs/MODULE_UI.md`
- `docs/MODULE_UI_TEMPLATE.json`

### Workspace fallback (`STATE_TEMPLATE.md` / `workspace.md`)

If `ui.json` is not present, ChattyCog falls back to a template-backed workspace editor:
- loads `workspace.md` if present
- otherwise seeds from `STATE_TEMPLATE.md` if present
- saves edits to `workspace.md` in the module folder

This fallback is especially useful for:
- CLI tools
- script-only modules
- research/process modules that need notes and state, but not a full custom GUI

### Suspend rundown ("what happened")

The suspend rundown is the canonical, cross-module "debrief" for the orchestrator and the long-term memory system.

When the user leaves a module tab (or closes it), ChattyCog emits a cold-log event with:
- `cat`: `module`
- `module_id`: the module you just left
- `event_type`: `suspend_rundown`
- `summary`: the module's suspend rundown text
- `tags`: includes `module_rundown`
- `payload_json` (optional): module metadata + a small snapshot preview (for indexing)

### Portable bridge (`bridge/status.json`) - recommended for hosted standalone modules

If your module keeps its own native UI or webview state, let the module report back through:

- `chattycog_gui/modules/<module>/bridge/status.json`
- `chattycog_gui/modules/<module>/bridge/log_sources.json` (optional; module-local logs ChattyCog may tail for context)

This keeps the module portable:

- standalone module keeps owning its own UI + state
- ChattyCog only reads the bridge file
- remove the bridge logic and the module still works outside ChattyCog

Important boundary:

- module bridge state is the **module-specific** sharing lane
- ChattyCog's generic **workflow bundle** lane is separate and is meant for whole-app setup
- this split keeps module state portable and prevents one broad setup bundle from pretending to be a module runtime sync

For hosted webviews, ChattyCog injects:

- `window.chattyCogBridge.available`
- `window.chattyCogBridge.updateStatus(payload)`
- `window.chattyCogBridge.clearStatus()`

For hosted native-window modules, ChattyCog sets environment variables:

- `CHATTYCOG_HOSTED=1`
- `CHATTYCOG_MODULE_DIR`
- `CHATTYCOG_BRIDGE_DIR`
- `CHATTYCOG_BRIDGE_STATUS`

See:
- `docs/MODULE_BRIDGE.md`
- `docs/MODULE_BRIDGE_TEMPLATE.json`
- `docs/MODULE_LOG_SOURCES_TEMPLATE.json`
- `docs/MODULE_NETWORK_CAPABILITIES_TEMPLATE.json`
- `docs/MODULE_BRIDGE_SNIPPETS.md`

### Network capability manifest (`network_capabilities.json`) - optional but recommended

If your module participates in LAN sharing, add:

- `chattycog_gui/modules/<module>/network_capabilities.json`

Example:

```json
{
  "features": [
    "shared_state_publish",
    "shared_state_receive",
    "room_aware"
  ],
  "notes": [
    "This module can publish and apply mirrored workflow state.",
    "It understands the shared-room lane but still stays standalone if the plug is removed."
  ]
}
```

Recognized features today:
- `shared_state_publish`
- `shared_state_receive`
- `workflow_bundle_send`
- `workflow_bundle_receive`
- `pack_send`
- `pack_receive`
- `lukewarm_context_publish`
- `lukewarm_context_receive`
- `room_aware`
- `multiplayer`
- `host_authoritative`

Optional `asset_lanes` let a module declare specific bridge inboxes for richer payloads.

Example:

```json
{
  "features": ["shared_state_receive"],
  "asset_lanes": [
    {
      "lane_id": "lesson_assets",
      "label": "Lesson Assets",
      "direction": "incoming",
      "delivery_mode": "bridge_inbox",
      "artifact_kinds": ["module_asset_file", "pack_file"],
      "accepted_content_types": ["text/markdown", "application/json", "application/octet-stream"],
      "max_bytes": 8388608,
      "replayable": true
    }
  ]
}
```

Use asset lanes when the module wants ChattyCog to hand it richer files or payloads through `bridge/incoming_assets/<lane_id>/` while still letting the standalone module decide when and how to import them.

`host_authoritative` matters most when a module also uses `room_aware` or `multiplayer`. It tells ChattyCog that hosted room sessions for that module should behave as "host leads, peers follow revisions" rather than as an unstructured shared room.

Why this exists:
- it keeps future LAN behavior explicit instead of assumed
- it lets ChattyCog disable or warn on actions the module has not intentionally declared
- it keeps standalone modules portable, because removing the file removes compatibility without breaking the tool itself

#### Auto-generation (recommended)

If **Preferences -> Auto-generate module suspend rundown on tab leave (Bookkeeper)** is enabled:
- ChattyCog first checks whether the module already supplied `bridge/status.json`.
- If the bridge contains a `summary`, ChattyCog uses that directly.
- If the bridge contains only a richer `snapshot`, the CPU-only Bookkeeper summarizes that.
- If `bridge/log_sources.json` exists, ChattyCog also tails the declared module-local logs and includes them in the Bookkeeper context bundle.
- If no bridge exists, ChattyCog falls back to its own form/workspace snapshot (plus last module AI output if any).
- The CPU-only Bookkeeper generates a short rundown automatically.
- The rundown is appended to the cold log and also updates:
  - `chattycog_gui/memory/departments.md`
  - `chattycog_gui/memory/departments.json`

These files represent the latest per-module status and are what the orchestrator reads for cross-module awareness.

### Module AI (demo)

If `ai_enabled: true`, the module tab also shows a minimal, local AI runner:
- model picker + generation params
- task input + output
- "Copy output -> suspend rundown"

This uses the same local llama.cpp runtime as the Chat tab.

## Orchestrator "Debrief" (department + lukewarm injection)

Before each orchestrator generation, ChattyCog reads:
- `chattycog_gui/memory/departments.md` (latest per-module rundowns)
- `chattycog_gui/memory/lukewarm.txt` (rolling recent activity summary)

and injects them into the system prompt under:
- `### DEPARTMENT STATUS UPDATES`
- `### RECENT ACTIVITY (LUKE WARM)`

## Cold Log Envelope (schemaless standard)

ChattyCog stores long-term memory in `chattycog_gui/memory/cold_log.jsonl`.

Recommended convention for module events:

Required intent fields:
- `module_id`: department/module identifier (use your manifest's `module_id`)
- `event_type`: what happened (short, stable string)
- `summary`: short human text (1 paragraph max)

Optional indexing fields:
- `tags`: list of strings to make filtering easier
- `payload_json`: free-form JSON as a string for extra structured data

## VRAM "Freeze" policy (current)

When a module tab is opened, ChattyCog pauses the orchestrator to free resources.

Current behavior:
- If the orchestrator is currently generating, it is allowed to finish the current response.
- After the response finishes, the orchestrator is paused while any module tab is active.
- While paused (or pause pending), the Chat input is disabled and new generations are blocked.

## Security / guardrails

Modules are treated as content folders for now.

If/when modules get code execution capabilities, the intended design is:
- module file I/O restricted to allowed directories (e.g. `Chatty_Sandbox/` and/or the module's own folder)
- no implicit access to arbitrary filesystem locations
- all operations routed through allowlist checks

Chat tab also supports a user-approved sandbox tool flow for the orchestrator (read/write/list within `Chatty_Sandbox/` only).
