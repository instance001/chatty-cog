use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use chattycog_gui::app_paths::{find_models_dir, find_runtime_windows_dir};
use chattycog_gui::llama_dyn::Llama;

fn main() -> Result<()> {
    let runtime_dir = find_runtime_windows_dir().context("locate runtime/windows/")?;
    let models_dir = find_models_dir().context("locate models/")?;

    let model = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| models_dir.join("tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf"));

    let llama = Llama::load(&runtime_dir)?;

    let cancel = AtomicBool::new(false);
    let mut out = String::new();
    llama.generate_chat(
        &model,
        "You are a helpful assistant.",
        "Say hello in one short sentence.",
        32,
        0.7,
        0.9,
        40,
        &cancel,
        |t| out.push_str(t),
    )?;

    // Best-effort: print without requiring valid UTF-8 pieces
    println!("{}", out.trim());

    // Ensure we didn't accidentally cancel
    cancel.store(false, Ordering::Relaxed);
    Ok(())
}
