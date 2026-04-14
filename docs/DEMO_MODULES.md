# Demo Modules (Onboarding Flow)

ChattyCog ships with a few stand-alone demo modules that are intentionally related, so you can see cross-module coordination through:
- Department Status Updates (`memory/departments.md`)
- Luke Warm (`memory/lukewarm.txt`)

Demo modules:
- `Work Schedule (Demo)` (`demo_work_schedule`)
- `Meal Planner (Demo)` (`demo_meal_planner`)
- `Errand Coach (AI Demo)` (`demo_errand_coach`) - includes a small per-module AI runner

They demonstrate:
- Module discovery + dynamic tabs (opened from the Modules menu)
- Module workspace surfaces (`ui.json` form or `workspace.md` editor)
- Hosted module UIs via `visual_load.json` (all three demo departments now ship as bundled webview examples)
- A real room-aware/multiplayer hosted module (`Work Schedule`) that reacts to shared-room sessions, host-led mirroring, and talking-stick control
- A living incoming-asset lane example (`Work Schedule`), where approved assets landing in `lesson_assets` can be previewed, imported into the planner, and marked consumed from inside the hosted module UI
- Automatic "on suspend" rundown -> cold log -> Department Status Updates
- Orchestrator reading `departments.md` + `lukewarm.txt` for cross-module coordination
- (For the AI demo) module-owned generation UI using the local runtime

## Where the demo modules live

They ship in:
- `chattycog_gui/modules/`

Each module includes:
- `manifest.json` (required for discovery)
- `HANDSHAKE.md` (how to use the department)
- `STATE_TEMPLATE.md` (optional fallback template for `workspace.md`)
- `ui.json` (recommended) + `state.json` output after you save

## Suggested onboarding exercise (15-20 minutes)

1) Start ChattyCog
- Run `cargo run --manifest-path chattycog_gui/Cargo.toml --bin chattycog_gui`

2) Open **Work Schedule (Demo)**
- Use **Modules -> Open: Work Schedule (Demo)**
- Fill in a few realistic values (timezone, work window, top 3, a couple deadlines).

3) Switch to **Meal Planner (Demo)**
- Open it from **Modules -> Open: Meal Planner (Demo)**
- Fill it in. Include your busiest days and note which days need low-prep meals.

4) Switch to **Errand Coach (AI Demo)**
- Open it from **Modules -> Open: Errand Coach (AI Demo)**
- Use the **Module AI (demo)** section:
  - pick a small model (defaults to the manifest `default_model` if present)
  - type a messy list of errands + constraints
  - click **Run**
  - optionally click **Copy output -> suspend rundown**

5) Switch back to **Chat**
- Ask the orchestrator a coordination question like:
  - "Given my schedule, meal plan constraints, and errands, propose a plan for tomorrow."
  - "Where should I add two 45-minute meal prep blocks to the work schedule?"
  - "Turn the errands plan into a checklist and write it to `Chatty_Sandbox/notes/errands.md`."

6) (Optional) Use sandbox tool requests
- If the orchestrator requests a sandbox read/write/list action, approve or reject it in the Chat UI.

## Troubleshooting

- If you don't see modules in the Modules menu:
  - Use **Modules -> Rescan modules**
  - Confirm the folders exist under `chattycog_gui/modules/` and contain `manifest.json`
