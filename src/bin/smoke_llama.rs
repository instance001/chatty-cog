use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use chattycog_gui::llama_dyn::Llama;

fn main() -> Result<()> {
    let runtime_dir = find_upwards_with_child("runtime")
        .context("locate runtime/")?
        .join("runtime")
        .join("windows");
    let models_dir = find_upwards_with_child("models")
        .context("locate models/")?
        .join("models");

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

fn find_upwards_with_child(child: &str) -> Result<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            starts.push(dir.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }

    for start in starts {
        let mut cur = Some(start.as_path());
        while let Some(dir) = cur {
            let candidate = dir.join(child);
            if candidate.is_dir() {
                return Ok(dir.to_path_buf());
            }
            cur = dir.parent();
        }
    }

    anyhow::bail!("could not locate `{child}` by searching upwards from exe/cwd");
}
