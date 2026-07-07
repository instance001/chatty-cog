use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn find_models_dir() -> Option<PathBuf> {
    find_upwards_with_child("models")
        .ok()
        .map(|root| root.join("models"))
}

pub fn find_modules_dir() -> Option<PathBuf> {
    find_upwards_with_child("modules")
        .ok()
        .map(|root| root.join("modules"))
}

pub fn find_runtime_windows_dir() -> Result<PathBuf> {
    let root = find_upwards_with_child("runtime")?;
    let path = root.join("runtime").join("windows");
    if path.is_dir() {
        Ok(path)
    } else {
        anyhow::bail!("runtime/windows not found at {}", path.display());
    }
}

pub fn find_default_logs_dir() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir() {
        let path = cwd.join("memory");
        if path.is_dir() {
            return Some(path);
        }
    }

    if let Ok(root) = find_upwards_with_child("memory") {
        let path = root.join("memory");
        if path.is_dir() {
            return Some(path);
        }
    }

    find_upwards_with_child("chattycog_gui")
        .ok()
        .map(|root| root.join("chattycog_gui").join("memory"))
        .filter(|path| path.is_dir())
}

pub fn find_sandbox_dir() -> Option<PathBuf> {
    find_upwards_with_child("Chatty_Sandbox")
        .ok()
        .map(|root| root.join("Chatty_Sandbox"))
}

pub fn find_or_create_sandbox_dir() -> Option<PathBuf> {
    if let Some(existing) = find_sandbox_dir() {
        return Some(existing);
    }

    let root = find_upwards_with_child("chattycog_gui")
        .ok()
        .or_else(|| std::env::current_dir().ok())?;
    let dir = root.join("Chatty_Sandbox");
    if std::fs::create_dir_all(&dir).is_ok() {
        Some(dir)
    } else {
        None
    }
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

    let headline = format!("- {}", truncate_chars(&lines[0], 160));
    let body = if lines.len() > 1 {
        truncate_chars(&lines[1..].join(" "), 320)
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
