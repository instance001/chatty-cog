# ChattyCog Module Templates

This folder holds starter modules that are safe to copy from.

It is outside `chattycog_gui/modules/`, so ChattyCog will not auto-discover these templates as live modules.

Need help choosing?

- `docs/MODULE_TEMPLATE_CHOOSER.md`
- `docs/MODULE_BUILDER_CHECKLIST.md`
- `docs/MODULE_PACKAGING_GUIDE.md`
- `docs/MODULE_REVIEW_RUBRIC.md`
- `docs/MODULE_RELEASE_NOTES_TEMPLATE.md`
- `docs/CHANGELOG_TEMPLATE.md`
- `docs/MODULE_SUBMISSION_TEMPLATE.md`

## Included

- `template_module/` - a standalone webview starter with the ChattyCog bridge already wired in
- `template_native_rust_module/` - a native Rust `eframe` starter with the ChattyCog bridge already wired in
- `template_python_module/` - a native Python `tkinter` starter with the ChattyCog bridge already wired in

## Typical workflow

1. Copy the starter you want
2. Paste it into `chattycog_gui/modules/`
3. Rename the copied folder
4. Update the copied module's `manifest.json`, `visual_load.json`, `network_capabilities.json`, `HANDSHAKE.md`, and its main UI file (`web/app.js` or `src/main.rs`)
5. In ChattyCog, use **Modules -> Rescan modules**
