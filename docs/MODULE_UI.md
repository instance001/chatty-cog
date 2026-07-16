# Module UI (`ui.json`) - Guide

This guide explains how to give your ChattyCog module a usable, native-feeling in-app UI **without writing any custom code in ChattyCog**.

ChattyCog can render a structured module surface from a file named `ui.json` inside your module folder and save the user's inputs to `state.json`.

## Quick start

1) Create a normal module (at minimum you need `manifest.json`)

```
chatty-cog/
  modules/
    your_module/
      manifest.json
      ui.json              <-- add this
```

2) Copy the template file:
- `docs/MODULE_UI_TEMPLATE.json`

into:
- `modules/your_module/ui.json`

3) Open the module in ChattyCog
- Use **Modules -> Open: <display_name>**
- Your module tab will show the form described by `ui.json`.

4) Save
- Click **Save** in the module tab.
- ChattyCog writes the user-entered values to:
  - `modules/your_module/state.json`

## What this is (and isn't)

This UI system is:
- A **declarative module surface** (section cards, text fields, checkboxes, number sliders, dropdowns, callouts, stats, and built-in action/file panels).
- A way for users to keep structured notes/state for a module.
- A way to create consistent "department state" that can be summarized into a "what happened" debrief.

This UI system is not:
- A full "code plugin" system (ChattyCog does not execute module code from `ui.json`).
- A security boundary (it's just UI + state persistence).

## File outputs

When your module provides `ui.json`, ChattyCog will create/update:
- `state.json` - persisted values for that form

If your module does **not** provide `ui.json`, ChattyCog falls back to a workspace editor and will create/update:
- `workspace.md` - freeform notes saved in the module folder

## `ui.json` format (schema)

Top level keys:
- `title` (optional string): heading shown in the module tab
- `description` (optional string): short text shown under the title
- `sections` (optional array): layout groups used to make the UI feel like a purpose-built module tab
- `fields` (array): list of form fields

### `sections`

Each section supports:
- `id` (optional string): stable grouping key for matching fields via `section`
- `title` (required string): card heading shown in the tab
- `description` (optional string): short helper text shown under the heading
- `blocks` (optional array): richer UI content rendered before any listed fields
- `fields` (optional array of field IDs): explicit field order for that card
- `sidebar` (optional bool): if `true`, ChattyCog renders the card in the right sidebar when there is enough width

If `sections` is omitted, ChattyCog still works:
- fields with the same `section` value are auto-grouped together
- otherwise fields fall back to a generic `Workspace` section

### `blocks`

Blocks let a module shape its own surface without writing custom ChattyCog code.

Supported block kinds:
- `field`
  - renders a specific field in-place
  - keys:
    - `kind: "field"`
    - `field` (required): field ID
- `text`
  - renders static explanatory text
  - keys:
    - `kind: "text"`
    - `title` (optional)
    - `text` (recommended)
- `markdown`
  - renders simple markdown-style text (static or from a field)
  - keys:
    - `kind: "markdown"`
    - `title` (optional)
    - `text` (optional): static markdown-ish content
    - `field` (optional): field ID to preview instead of static text
    - `empty` (optional): fallback text when the source is blank
- `callout`
  - renders a highlighted helper/instruction card
  - keys:
    - `kind: "callout"`
    - `title` (optional)
    - `text` (recommended)
    - `tone` (optional): `info`, `success`, `warning`, `error`
- `stat`
  - renders a compact read-only summary of a field
  - keys:
    - `kind: "stat"`
    - `field` (required): field ID
    - `label` (optional): title shown above the value
    - `empty` (optional): fallback text when the field is blank
- `actions`
  - renders built-in module action buttons
  - keys:
    - `kind: "actions"`
    - `actions` (required array)
- `progress`
  - renders a single numeric field as a progress bar
  - keys:
    - `kind: "progress"`
    - `field` (required): numeric field ID
    - `label` (optional)
    - `min` / `max` (optional): override the field's numeric range
    - `empty` (optional)
- `record`
  - renders a lightweight key/value summary from selected fields
  - keys:
    - `kind: "record"`
    - `title` (optional)
    - `fields` (required array): field IDs to show
    - `empty` (optional)
- `checklist`
  - renders simple checklist-style lines from a field or module-local text file
  - keys:
    - `kind: "checklist"`
    - `title` (optional)
    - `field` (optional): multiline field with lines like `- [ ]`, `- [x]`, `- [~]`, bullets, or numbered steps
    - `path` (optional): relative path to a module-local text file
    - `max_rows` (optional): defaults to `12`
    - `searchable` (optional): defaults to `true`; shows an in-block filter bar once the list grows
    - `filter_placeholder` (optional): custom hint text for the filter box
    - `filter_presets` (optional array): named quick-view buttons like `[{ "label": "Done", "query": "done" }]`
    - `empty` (optional)
- `timeline`
  - renders an ordered event/history list from a field or module-local text file
  - keys:
    - `kind: "timeline"`
    - `title` (optional)
    - `field` (optional): multiline field with lines like `09:00 | Deep work`, `[2026-03-15] Note`, or `Step - result`
    - `path` (optional): relative path to a module-local text file
    - `max_rows` (optional): defaults to `10`
    - `searchable` (optional): defaults to `true`; shows an in-block filter bar once the list grows
    - `filter_placeholder` (optional): custom hint text for the filter box
    - `filter_presets` (optional array): named quick-view buttons for common timeline views
    - `empty` (optional)
- `kanban`
  - renders a lightweight board from a field or module-local text file
  - keys:
    - `kind: "kanban"`
    - `title` (optional)
    - `field` (optional): multiline field with lines like `[Doing] Draft handoff`, `Review | Test build`, checklist items, or plain notes
    - `path` (optional): relative path to a module-local text file
    - `lanes` (optional array): preferred lane order such as `["To Do", "Doing", "Review", "Done"]`
    - `max_rows` (optional): defaults to `18`
    - `searchable` (optional): defaults to `true`; shows an in-block filter bar once the board grows
    - `filter_placeholder` (optional): custom hint text for the filter box
    - `filter_presets` (optional array): named quick-view buttons such as blocked/review/done
    - `empty` (optional)
- `table`
  - renders a lightweight table from multiline field content or a module-local file
  - keys:
    - `kind: "table"`
    - `title` (optional)
    - `field` (optional): field containing CSV, TSV, semicolon, or pipe-delimited rows
    - `path` (optional): relative path to a module-local text file with table rows
    - `has_header` (optional): defaults to `true`
    - `max_rows` (optional): defaults to `8`
    - `searchable` (optional): defaults to `true`; shows an in-block filter bar once the table grows
    - `filter_placeholder` (optional): custom hint text for the filter box
    - `filter_presets` (optional array): named quick-view buttons for common row filters
    - `empty` (optional)
- `bar_chart`
  - renders several numeric fields as a simple bar-style chart
  - keys:
    - `kind: "bar_chart"`
    - `title` (optional)
    - `fields` (required array): numeric field IDs
    - `min` / `max` (optional): shared override range for all bars
    - `empty` (optional)
- `dependency_graph`
  - renders a staged dependency flow from a field or module-local text file
  - keys:
    - `kind: "dependency_graph"`
    - `title` (optional)
    - `field` (optional): multiline field with lines like `Prep data -> Run eval -> Export report`
    - `path` (optional): relative path to a module-local text file
    - `max_rows` (optional): defaults to `16`
    - `searchable` (optional): defaults to `true`; shows an in-block filter bar once the graph grows
    - `filter_placeholder` (optional): custom hint text for the filter box
    - `filter_presets` (optional array): named quick-view buttons for common dependency views
    - `empty` (optional)
- `tabs`
  - renders a nested tab strip with pane-specific content
  - keys:
    - `kind: "tabs"`
    - `id` (recommended): stable UI ID so the selected tab stays consistent while the module is open
    - `title` (optional)
    - `tabs` (required array): pane definitions
    - `view_presets` (optional array): named pane lenses such as `[{ "label": "Ops View", "pane_ids": ["ops_tab"] }]`
- `split`
  - renders a nested split workspace using panes
  - keys:
    - `kind: "split"`
    - `title` (optional)
    - `direction` (optional): `horizontal` (default) or `vertical`
    - `columns` (required array): pane definitions
    - `view_presets` (optional array): named workspace lenses that show only selected panes
- `accordion`
  - renders a stack of collapsible work areas
  - keys:
    - `kind: "accordion"`
    - `id` (recommended): stable UI ID for the accordion
    - `title` (optional)
    - `panes` (required array): pane definitions
    - `view_presets` (optional array): named lenses for focused accordion views
- `inspector`
  - renders a tighter, sidebar-friendly collapsible stack
  - keys:
    - `kind: "inspector"`
    - `id` (recommended): stable UI ID for the inspector
    - `title` (optional)
    - `panes` (required array): pane definitions
    - `view_presets` (optional array): named lenses for focused inspector views
- `file_list`
  - renders a safe file list from inside the module folder
  - keys:
    - `kind: "file_list"`
    - `title` (optional)
    - `path` (optional): relative path inside the module folder; `.` means the module root
    - `max_entries` (optional): default `8`
    - `searchable` (optional): defaults to `true`; shows an in-block filter bar once the list grows
    - `filter_placeholder` (optional): custom hint text for the filter box
    - `filter_presets` (optional array): named quick-view buttons for common file subsets
    - `empty` (optional): fallback text if nothing is there
- `artifact_preview`
  - renders a safe preview of a module-local file or folder
  - keys:
    - `kind: "artifact_preview"`
    - `title` (optional)
    - `path` (optional): relative path inside the module folder
    - `field` (optional): field containing one or more relative file paths
    - `max_lines` (optional): preview height for text files
    - `empty` (optional): fallback text when no previewable artifact exists
- `separator`
  - renders a divider
  - keys:
    - `kind: "separator"`
- `spacer`
  - renders blank vertical space
  - keys:
    - `kind: "spacer"`
    - `points` (optional): height in points

Supported built-in actions:
- `save`
- `reload`
- `open_folder`
- `open_readme`
- `open_manual`
- `open_handshake`
- `open_state`
- `open_manifest`

### Pane definitions (`tabs` / `split`)

Nested panes support:
- `id` (optional string): stable identifier for the pane
- `title` (required string): label shown on the tab or pane heading
- `description` (optional string): short helper text
- `summary` (optional string): compact text appended to the collapsed header
- `summary_field` (optional string): use the current field value as the compact header summary
- `blocks` (optional array): any supported blocks, including nested layouts
- `fields` (optional array): fields shown in that pane
- `weight` (optional number): used by `split` to size panes relative to each other
- `default_open` (optional bool): whether the pane starts open the first time it is shown

Notes:
- `tabs` remembers the selected pane while the module tab stays open.
- `split` automatically stacks vertically if the ChattyCog window gets too narrow for a clean side-by-side layout.
- `accordion` is good for large, sequential workspaces where only one or two parts need to be open at once.
- `inspector` is ideal for sidebars, diagnostics, artifacts, and handoff details that should stay available but unobtrusive.
- `view_presets` automatically gets a `Default` reset button and lets modules ship focused workspace lenses like `Ops View`, `Handoff View`, or `Debug View`.

Each field supports:
- `id` (required string): stable key saved into `state.json`
- `label` (required string): text shown to the user
- `kind` (optional string): one of:
  - `singleline`
  - `multiline` (default if omitted/unknown)
  - `number`
  - `bool`
  - `choice`
- `placeholder` (optional string): hint text for text fields
- `help` (optional string): smaller supporting text shown under the label
- `section` (optional string): which section/card this field belongs to
- `rows` (optional number): multiline height (2-24 recommended)
- `min` / `max` (optional numbers): for `number`
  - if both are present, ChattyCog renders a slider
  - if missing, ChattyCog renders a draggable number input
- `options` (optional array of strings): for `choice`

## Best practices

- Use `sections` to make the module feel like its own workspace instead of one long generic form.
- Keep the "main work" in normal sections and put status/artifacts/handoff fields in a `sidebar: true` section.
- Use `blocks` for instruction cards, quick status summaries, and safe file panels instead of forcing everything into editable fields.
- Use `record` when you want a clean read-only summary without duplicating data entry fields.
- Use `checklist` when the same raw text should be editable and also readable as a task list.
- Use `timeline` for schedules, experiment logs, chronological notes, and operational traces.
- Use `kanban` when a module benefits from lane-based work tracking without needing a fully custom board UI.
- Use `table` when a field naturally contains structured rows such as CSV/TSV/pipe data, tool runs, schedules, or experiment logs.
- Use `dependency_graph` when the important thing is sequencing or prerequisite flow between tasks, stages, or artifacts.
- Leave `searchable` enabled for bigger, long-lived modules so people can stay oriented without opening extra tabs.
- Use `filter_presets` for the 2-5 views people will keep revisiting; ChattyCog automatically adds an `All` button alongside them.
- Keep `id` values stable once users start using the module (changing IDs "loses" old saved values).
- Use short, human labels. Put details in `description`, `help`, or placeholders.
- Use `choice` for standard categories (budget level, difficulty, department, status, etc.).
- Use `number` with `min`/`max` when you want a slider instead of free typing.
- Don't put secrets into `state.json` (it's plain text on disk).

## Native-feeling module surfaces

To make a module tab feel purpose-built:
- define a small number of sections (2-5 is usually enough)
- use one "overview" section for the main task
- use one sidebar section for status, artifacts, and handoff
- use `tabs` when the user needs to switch between related sub-workspaces without opening more top-level ChattyCog tabs
- use `split` when two areas should stay visible together, such as a form beside a preview or a tool panel beside artifacts
- use `accordion` when the module has multiple deep panels but only a few should be expanded at a time
- use `inspector` in sidebars for compact, collapsible status and artifact panels
- use `view_presets` on nested layouts when different roles need different workspace lenses without duplicating sections
- use `callout`, `markdown`, `stat`, `record`, `checklist`, `timeline`, `kanban`, `table`, `progress`, `bar_chart`, `dependency_graph`, `actions`, `artifact_preview`, and `file_list` blocks to make the module feel like a real department surface
- use `filter_placeholder` when a block needs a more specific hint like "Find ticket, lane, or owner..."
- use short preset labels like `Blocked`, `Review`, `Done`, `Exports`, or `Eval`
- add `help` text only where it genuinely unblocks a first-time user
- keep multiline fields focused so the auto-generated suspend rundown stays readable

## How this connects to memory / orchestration

- When the user leaves a module tab (or closes it), ChattyCog emits a "suspend rundown" event to the Bookkeeper (cold log).
- If **Models -> Auto-generate module suspend rundown on tab leave (Bookkeeper)** is enabled, the Bookkeeper auto-generates this rundown from a snapshot of:
  - the module form (`state.json`) or workspace (`workspace.md`)
  - last module AI output (if any)
- The latest per-module debriefs are written to:
  - `memory/departments.md`
  - `memory/departments.json`
- The orchestrator reads `departments.md` before each Chat response, so it stays cross-module aware.
