# Chat UI Smoke Checklist

Use this quick pass after touching the Chat tab layout, composer behavior, Luke Warm panel, or sandbox approval UI.

The goal is not exhaustive QA. It is a fast regression catch for the chat screen's most fragile behaviors.

## Before You Start

1. Build and launch the debug app:

```bash
cargo build --bin chattycog_gui
target\debug\chattycog_gui.exe
```

2. Open the `Chat` tab.

3. If the splash screen appears, dismiss it and wait for the main chat layout.

## Smoke Pass

### 1. Baseline layout

- Confirm `Hot Memory`, `Chat`, and `Luke Warm` are all visible at the same time.
- Confirm the right panel is not pushed off-screen at the default window size.
- Confirm the top chat header still shows runtime status, model controls, voice summary, and ECG without obvious overlap.

### 2. Composer focus

- Click into the composer once.
- Type a long message continuously.
- Confirm typing does not require reclicking partway through.
- Pause for 2 to 3 seconds, then continue typing.
- Confirm focus stays in the composer after the pause.
- Press `Shift+Enter` and confirm a new line is inserted.
- Press `Enter` and confirm the message sends.

### 3. Transcript growth

- Send 2 to 3 long messages in a row.
- Confirm the center transcript grows without widening the overall chat workspace.
- Confirm `Luke Warm` remains visible after repeated long sends.
- Confirm the composer is still usable immediately after sending.
- If no GGUF is selected, confirm the expected error bubble still stays inside the transcript column and does not push the layout wider.

### 4. Composer persistence

- After sending a long message, type again without clicking back into the composer.
- Confirm the input still has focus and accepts typing.
- Confirm `Send` and `Interrupt` buttons still align cleanly with the input row.

### 5. Luke Warm readability

- Confirm the `Luke Warm` helper text wraps cleanly.
- Confirm the summary content wraps instead of clipping horizontally.
- Confirm long summary lines stay readable in the visible panel width.

### 6. Luke Warm refresh

- Leave the app idle on `Chat` or `Logs` for at least 3 to 5 seconds.
- Confirm the Luke Warm panel still refreshes on its own polling cadence and does not stall once the UI is otherwise idle.
- If you are doing a deeper manual check, temporarily change `memory/lukewarm.txt` and confirm the visible panel updates on the next refresh cycle, then restore the original file.

### 7. Hot Memory readability

- Pin or generate enough items to populate `Hot Memory`.
- Confirm entries wrap instead of forcing the panel wider.
- Confirm the panel footer buttons remain visible.

### 8. Sandbox action block

- Enable `Sandbox task`.
- Enter a target path like `notes/request.md`.
- Confirm the helper copy stays readable and does not overlap other controls.
- If pending sandbox actions appear, confirm the approval block remains readable and does not shove the composer off-screen.

### 9. Tab return

- Switch to another top tab such as `Logs` or `Sandbox`.
- Return to `Chat`.
- Confirm the three-column layout restores correctly.
- Confirm the composer and side panels still look stable after the tab switch.

## Pass / Fail Rule

Treat the smoke pass as failed if any of these happen:

- the right panel drifts off-screen
- the composer loses focus during ordinary typing
- the composer loses focus after a short pause and resumed typing
- long content causes horizontal creep
- Luke Warm stops refreshing unless the user manually interacts with the UI
- wrapped helper text clips or overlaps controls
- sandbox approval UI makes the composer or side panels unusable

## When To Run It

Run this checklist after changes to:

- `src/chat_ui.rs`
- `src/chat_actions.rs`
- `src/sandbox_ops.rs`
- `src/sandbox_editor.rs`
- any layout-affecting chat code in `src/main.rs`

Also run it after broad egui upgrades or style changes, even if the chat code itself was untouched.
