# Module Visual Load-In

Use `visual_load.json` when a module already has its **own standalone UI** and you want ChattyCog to host that exact UI inside the module tab.

Bundled repo examples:
- `chattycog_gui/modules/demo_work_schedule/` - static-file `webview`
- `chattycog_gui/modules/demo_meal_planner/` - static-file `webview`
- `chattycog_gui/modules/demo_errand_coach/` - static-file `webview`
- `chattycog_gui/modules/new_janet_school/` - hosted research dashboard `webview`
- `chattycog_gui/modules/llm_tweaker/` - hosted `native_window`
- `chattycog_gui/modules/chattyfactory-module/` - hosted `native_window`

This keeps the module self-contained while letting ChattyCog:
- discover it automatically
- launch it from the tab
- build it first if the module advertises a build command
- keep the normal handshake / suspend-rundown bridge alongside it

Pair this with:
- `docs/MODULE_BRIDGE.md`
- `docs/MODULE_BRIDGE_SNIPPETS.md`
- `docs/MODULE_BUILDER_CHECKLIST.md`
- `docs/MODULE_PACKAGING_GUIDE.md`
- `docs/MODULE_REVIEW_RUBRIC.md`

That gives you the full loop:
- `manifest.json` = discovery
- `visual_load.json` = host the real UI
- `bridge/status.json` = let the standalone module report back to ChattyCog

## Folder layout

```text
chattycog_gui/
  modules/
    your_module/
      manifest.json
      visual_load.json
      ...
```

`manifest.json` is still the discovery anchor. `visual_load.json` is the optional visual load-in block.

## Current support

Supported today:
- `native_window` on Windows
- `webview` on Windows (hosted through ChattyCog's bundled WebView helper)

Current behavior:
- `native_window`: ChattyCog launches the standalone app and docks that real desktop window into the tab.
- `webview`: ChattyCog hosts the module's real browser-style dashboard inside the tab.
- no `visual_load.json`: ChattyCog falls back to the module's `ui.json` / workspace surface.

## What users will actually see

There are three visual paths in ChattyCog:

### 1) Docked native app

This is what modules like `LLM Tweaker` and `ChattyFactory` use.

What it means:
- the module already has its own desktop window
- ChattyCog launches that real app
- ChattyCog visually docks that real app into the tab

Result:
- it looks the closest to "the module running normally, just inside ChattyCog"

### 2) Docked web dashboard

This is what modules like `Errand Coach`, `Meal Planner`, `Work Schedule`, and `Janet School` use.

What it means:
- the module's real UI is HTML/CSS/JS
- ChattyCog hosts that browser-style UI in a webview inside the tab
- the module is still using its own actual dashboard, not a generic placeholder

Result:
- it may look more like a form or dashboard because that is what the module's real web UI is
- this is still the module's own hosted UI, just in browser form instead of desktop-window form

### 3) ChattyCog fallback workspace

Use this when a module is headless, CLI-only, or simply does not ship a standalone GUI.

What it means:
- no `visual_load.json`
- ChattyCog renders the module through `ui.json` or the workspace editor fallback

Result:
- this is the "middle ground" surface
- the module still works in ChattyCog, but ChattyCog is supplying the visual shell

## Why the split exists

ChattyCog supports these three paths on purpose:

- some modules are true desktop apps and should keep that exact experience
- some modules are really web dashboards and should keep that exact experience
- some modules are tools, scripts, or workflows with no GUI at all, so ChattyCog needs to provide a practical surface

This keeps the ecosystem simple:

- builders who already have a UI can keep it
- builders who only have logic or CLI tools can still become compatible without building a full app shell first
- users still get a consistent tabbed workflow either way

The simple rule:
- `native_window` = host the module's real desktop UI
- `webview` = host the module's real browser UI
- fallback workspace = ChattyCog-provided UI for modules without their own hosted surface

## Two hosting styles

### 1) Native desktop window

Use this when your module already launches a real desktop window (`eframe`, `tao`, `tkinter`, `PySide`, etc.).

```json
{
  "kind": "native_window",
  "auto_launch": true,
  "window_title_contains": "LLM Tweaker",
  "launch": {
    "program": "target/debug/llm_tweaker.exe",
    "cwd": "."
  }
}
```

### 2) Webview

Use this when your module UI is browser-style and should be hosted inside ChattyCog as a docked webview.

Static-file example:

```json
{
  "kind": "webview",
  "auto_launch": true,
  "title": "Research Dashboard",
  "file": "web/index.html"
}
```

Local-server example:

```json
{
  "kind": "webview",
  "auto_launch": true,
  "title": "Research Dashboard",
  "url": "http://127.0.0.1:4173",
  "serve": {
    "program": "npm",
    "args": ["run", "preview"],
    "cwd": "."
  },
  "serve_wait_ms": 1500
}
```

## `visual_load.json` schema

Example:

```json
{
  "kind": "native_window",
  "auto_launch": true,
  "window_title_contains": "LLM Tweaker",
  "notes": "Optional human note shown above the hosted UI.",
  "build": {
    "program": "cargo",
    "args": ["build"],
    "cwd": "."
  },
  "launch": {
    "program": "target/debug/llm_tweaker.exe",
    "cwd": "."
  }
}
```

Fields:
- `kind` - use `native_window` or `webview`
- `auto_launch` - if `true`, ChattyCog tries to launch the module when the tab opens
- `title` - optional hosted window title (especially useful for `webview`)
- `url` - webview target URL
- `file` - module-local HTML file to open in a hosted webview
- `window_title_contains` - helps ChattyCog identify the correct top-level window to dock
- `notes` - optional user-facing note in the host toolbar
- `build` - optional command shown as **Build UI**
- `launch` - command used to start the standalone module UI
- `serve` - optional background command started before a hosted webview opens
- `serve_wait_ms` - optional delay before the hosted webview opens (useful for local dev servers)

Command fields:
- `program` - executable or command name
- `args` - optional argument list
- `cwd` - optional working directory, relative to the module folder
- `env` - optional environment variables

## Good defaults

For Rust/eframe modules:
- `window_title_contains` should match the string passed to `eframe::run_native(...)`
- `build.program` is usually `cargo`
- `launch.program` is usually `target/debug/<your_app>.exe`

For Python GUI modules:
- point `launch.program` at `py`, `python`, or `pythonw`
- put the script path in `args`
- make sure the script actually opens a GUI window

For hosted webviews:
- use `file` for static HTML/JS/CSS
- use `url` + optional `serve` for local dashboard/dev-server style modules
- prefer a stable local URL such as `http://127.0.0.1:4173`

## Tips

- Prefer a specific `window_title_contains`; it makes docking more reliable.
- Keep `launch.program` relative to the module folder when possible.
- If the module needs a one-time build, advertise `build` so users can do it from the tab.
- For `webview`, Windows needs WebView2 available (normally already present on Windows 11).
- If your module is headless or CLI-only, do **not** add `visual_load.json`; let ChattyCog use the regular workspace surface instead.
- If your browser-style module looks like a structured dashboard inside ChattyCog, that is still its real hosted UI, not the old placeholder screen.

## Relationship to the handshake

`visual_load.json` only answers:
- how to visually launch the module
- how ChattyCog should host it

Your normal module handshake still lives in:
- `manifest.json`
- `HANDSHAKE.md`
- optional `ui.json`

Those files still govern:
- identity
- suspend rundown / department context
- structured state surfaces
- cross-module coordination

## Hosted bridge hooks

When ChattyCog hosts a module UI, the module can stay fully standalone and only use a tiny optional bridge.

### Hosted native-window modules

ChattyCog sets:
- `CHATTYCOG_HOSTED=1`
- `CHATTYCOG_MODULE_DIR`
- `CHATTYCOG_BRIDGE_DIR`
- `CHATTYCOG_BRIDGE_STATUS`

If your app sees those variables, it can write `bridge/status.json`.
If it ignores them, it still runs standalone normally.

### Hosted webview modules

ChattyCog injects:

```js
window.chattyCogBridge.available
window.chattyCogBridge.updateStatus(payload)
window.chattyCogBridge.clearStatus()
```

So a hosted webview can do:

```js
window.chattyCogBridge?.updateStatus({
  module_id: "your_module",
  summary: "Short handoff text.",
  snapshot: "Longer state dump."
});
```

Outside ChattyCog, that object simply does not exist, so the module keeps working in a normal browser too.
