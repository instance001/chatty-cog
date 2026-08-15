use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const APP_BASE_ENV: &str = "CHATTYCOG_BASE_PATH";

const APP_DIRS: &[&str] = &[
    "models",
    "runtime",
    "runtime/windows",
    "modules",
    "memory",
    "logs",
    "config",
    "Chatty_Sandbox",
    "Chatty_Sandbox/scratchpad",
    "network_inbox",
    "network_inbox/workflow_states",
    "network_inbox/workflow_bundles",
    "network_inbox/lukewarm_context",
    "network_inbox/applied_lukewarm_context",
    "network_inbox/file_transfers",
    "network_inbox/file_transfers/payloads",
    "network_inbox/applied_file_transfers",
    "network_recovery",
    "network_recovery/module_session_payloads",
    "network_trust_exports",
    "network_trust_imports",
];

pub fn find_models_dir() -> Option<PathBuf> {
    ensure_app_dirs().ok().map(|root| root.join("models"))
}

pub fn find_modules_dir() -> Option<PathBuf> {
    ensure_app_dirs().ok().map(|root| root.join("modules"))
}

pub fn find_runtime_windows_dir() -> Result<PathBuf> {
    let root = ensure_app_dirs()?;
    let path = root.join("runtime").join("windows");
    if path.is_dir() {
        Ok(path)
    } else {
        anyhow::bail!("runtime/windows not found at {}", path.display());
    }
}

pub fn find_default_logs_dir() -> Option<PathBuf> {
    ensure_app_dirs().ok().map(|root| root.join("memory"))
}

pub fn find_sandbox_dir() -> Option<PathBuf> {
    ensure_app_dirs()
        .ok()
        .map(|root| root.join("Chatty_Sandbox"))
}

pub fn find_or_create_sandbox_dir() -> Option<PathBuf> {
    find_sandbox_dir()
}

pub fn app_base_dir() -> Result<PathBuf> {
    if let Ok(value) = std::env::var(APP_BASE_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().filter(|path| path.is_dir()) {
            if dir.join("Cargo.toml").is_file() && dir.join("src").is_dir() {
                return Ok(dir.to_path_buf());
            }
        }

        if let Some(dir) = exe.parent() {
            return Ok(dir.to_path_buf());
        }
    }

    std::env::current_dir().context("current_dir")
}

pub fn ensure_app_dirs() -> Result<PathBuf> {
    let base = app_base_dir()?;
    std::fs::create_dir_all(&base).with_context(|| format!("mkdir {}", base.display()))?;

    for rel in APP_DIRS {
        let dir = base.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    }

    Ok(base)
}

pub fn read_lukewarm_from_logs_dir(logs_dir: Option<&Path>) -> Result<String> {
    let Some(dir) = resolve_logs_dir(logs_dir) else {
        return Ok(String::new());
    };
    let path = dir.join("lukewarm.txt");
    if !path.is_file() {
        return Ok(String::new());
    }
    Ok(sanitize_lukewarm_summary_for_ui(&read_text_file(
        &path, 200_000,
    )?))
}

pub fn read_departments_from_logs_dir(logs_dir: Option<&Path>) -> Result<String> {
    let Some(dir) = resolve_logs_dir(logs_dir) else {
        return Ok(String::new());
    };
    let path = dir.join("departments.md");
    if !path.is_file() {
        return Ok(String::new());
    }
    read_text_file(&path, 200_000)
}

pub fn read_gguf_architecture(path: &Path) -> Result<Option<String>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;

    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)
        .with_context(|| format!("read {}", path.display()))?;
    if &magic != b"GGUF" {
        return Ok(None);
    }

    let version = read_u32(&mut file)?;
    if !(2..=3).contains(&version) {
        return Ok(None);
    }

    let _tensor_count = read_u64(&mut file)?;
    let kv_count = read_u64(&mut file)?;

    for _ in 0..kv_count {
        let key = read_gguf_string(&mut file)?;
        let value_type = read_u32(&mut file)?;
        if key == "general.architecture" {
            if value_type == 8 {
                return Ok(Some(read_gguf_string(&mut file)?));
            }
            skip_gguf_value(&mut file, value_type)?;
            return Ok(None);
        }
        skip_gguf_value(&mut file, value_type)?;
    }

    Ok(None)
}

fn resolve_logs_dir(logs_dir: Option<&Path>) -> Option<PathBuf> {
    logs_dir
        .map(Path::to_path_buf)
        .or_else(find_default_logs_dir)
}

fn read_text_file(path: &Path, max_bytes: usize) -> Result<String> {
    use std::io::Read;

    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = Vec::new();
    file.take(max_bytes as u64)
        .read_to_end(&mut buf)
        .with_context(|| format!("read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn read_u32(file: &mut std::fs::File) -> Result<u32> {
    let mut buf = [0_u8; 4];
    use std::io::Read;
    file.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64(file: &mut std::fs::File) -> Result<u64> {
    let mut buf = [0_u8; 8];
    use std::io::Read;
    file.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_gguf_string(file: &mut std::fs::File) -> Result<String> {
    let len = read_u64(file)? as usize;
    let mut buf = vec![0_u8; len];
    use std::io::Read;
    file.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

fn skip_gguf_value(file: &mut std::fs::File, value_type: u32) -> Result<()> {
    match value_type {
        0 | 1 | 7 => skip_bytes(file, 1),
        2 | 3 => skip_bytes(file, 2),
        4 | 5 | 6 => skip_bytes(file, 4),
        10 | 11 | 12 => skip_bytes(file, 8),
        8 => {
            let len = read_u64(file)?;
            skip_bytes(file, len)
        }
        9 => {
            let element_type = read_u32(file)?;
            let len = read_u64(file)?;
            if element_type == 8 {
                for _ in 0..len {
                    let str_len = read_u64(file)?;
                    skip_bytes(file, str_len)?;
                }
                Ok(())
            } else {
                let element_size = gguf_primitive_size(element_type)
                    .with_context(|| format!("unsupported GGUF array type {element_type}"))?;
                skip_bytes(file, len.saturating_mul(element_size as u64))
            }
        }
        other => anyhow::bail!("unsupported GGUF value type {other}"),
    }
}

fn gguf_primitive_size(value_type: u32) -> Result<usize> {
    match value_type {
        0 | 1 | 7 => Ok(1),
        2 | 3 => Ok(2),
        4 | 5 | 6 => Ok(4),
        10 | 11 | 12 => Ok(8),
        _ => anyhow::bail!("unsupported GGUF primitive type {value_type}"),
    }
}

fn skip_bytes(file: &mut std::fs::File, len: u64) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};

    if len == 0 {
        return Ok(());
    }

    match file.seek(SeekFrom::Current(len as i64)) {
        Ok(_) => Ok(()),
        Err(_) => {
            let mut limited = file.take(len);
            std::io::copy(&mut limited, &mut std::io::sink())?;
            Ok(())
        }
    }
}

fn sanitize_lukewarm_summary_for_ui(raw: &str) -> String {
    let cleaned = raw
        .replace("<bullet>", "")
        .replace("<paragraph>", "")
        .replace("```", "")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return String::new();
    }

    let lines: Vec<String> = cleaned
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.trim_start_matches(['-', '*', '•', ' '])
                .trim()
                .to_string()
        })
        .filter(|line| !looks_like_lukewarm_scaffolding(line))
        .filter(|line| !line.is_empty())
        .collect();

    if lines.is_empty() {
        return String::new();
    }

    let headline = format!("- {}", truncate_chars(&lines[0], 260));
    let body = if lines.len() > 1 {
        truncate_chars(&lines[1..].join(" "), 900)
    } else {
        String::new()
    };

    if body.is_empty() {
        headline
    } else {
        format!("{headline}\n{body}")
    }
}

fn looks_like_lukewarm_scaffolding(line: &str) -> bool {
    let normalized = line.trim().to_ascii_lowercase();
    [
        "okay, let's",
        "ok, let's",
        "first, i need",
        "i need to parse",
        "looking at the activity",
        "the key points",
        "the bullet should",
        "the paragraph needs",
        "the user wants",
    ]
    .iter()
    .any(|pattern| normalized.starts_with(pattern))
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut it = s.chars();
    let mut out = String::new();
    for _ in 0..max_chars {
        match it.next() {
            Some(ch) => out.push(ch),
            None => return out,
        }
    }
    if it.next().is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let previous = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(previous) = &self.previous {
                    std::env::set_var(self.key, previous);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn lukewarm_ui_sanitizer_keeps_long_body_context() {
        let body =
            "The rolling summary should stay readable across repeated bookkeeper updates. "
                .repeat(8);
        let raw = format!("- Summary refreshed.\n{body}Next action remains visible.");

        let sanitized = sanitize_lukewarm_summary_for_ui(&raw);

        assert!(sanitized.contains("Summary refreshed"));
        assert!(sanitized.contains("Next action remains visible"));
        assert!(
            sanitized.chars().count() > 500,
            "UI sanitizer over-truncated the rolling summary: {sanitized}"
        );
    }

    #[test]
    fn ensure_app_dirs_bootstraps_binary_first_run_layout() {
        let base =
            std::env::temp_dir().join(format!("chattycog-first-run-test-{}", std::process::id()));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }

        let _guard = EnvGuard::set(APP_BASE_ENV, &base);
        let root = ensure_app_dirs().unwrap();

        assert_eq!(root, base);
        for rel in APP_DIRS {
            assert!(
                base.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
                    .is_dir(),
                "missing first-run directory: {rel}"
            );
        }

        std::fs::remove_dir_all(&base).unwrap();
    }
}
