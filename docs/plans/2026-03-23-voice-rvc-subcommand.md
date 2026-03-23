# `kotoba voice rvc` Subcommand Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add `kotoba voice rvc {list,set,off}` subcommands so users can manage RVC models without memorizing long directory names or risking typos in `config set`.

**Architecture:** Extend the existing `VoiceAction` enum with a new `Rvc` variant containing `VoiceRvcAction` subcommands. The `list` command scans `~/.kotoba/models/rvc/` for directories containing `model.pth`. The `set` command validates the model exists and supports case-insensitive fuzzy substring matching. All logic lives in `src/cli/voice.rs`.

**Tech Stack:** clap (CLI parsing), serde_json (output), snafu (errors), std::fs (directory scanning)

---

### Task 1: Add CLI enum variants

**Files:**
- Modify: `src/cli/mod.rs`

**Step 1: Add `VoiceRvcAction` enum and wire into `VoiceAction`**

Add after the `VoiceToneAction` enum (~line 267):

```rust
/// RVC model management subcommands.
#[derive(Subcommand)]
pub enum VoiceRvcAction {
    /// List downloaded RVC models
    List,
    /// Set the active RVC model (supports fuzzy matching)
    Set {
        /// Model name or substring (e.g. "miku")
        name: String,
    },
    /// Disable RVC voice conversion
    Off,
}
```

Add a new variant to `VoiceAction` (after `Tone`):

```rust
    /// Manage RVC voice conversion models
    Rvc {
        #[command(subcommand)]
        action: VoiceRvcAction,
    },
```

**Step 2: Write CLI parsing tests**

Add to `src/cli/mod.rs` tests:

```rust
#[test]
fn voice_rvc_list_command_parses() {
    let cli = Cli::try_parse_from(["kotoba", "voice", "rvc", "list"])
        .expect("parse should succeed");
    let Command::Voice { action } = cli.command else {
        panic!("expected voice command");
    };
    let VoiceAction::Rvc { action } = action else {
        panic!("expected voice rvc command");
    };
    assert!(matches!(action, VoiceRvcAction::List));
}

#[test]
fn voice_rvc_set_command_parses() {
    let cli = Cli::try_parse_from(["kotoba", "voice", "rvc", "set", "miku"])
        .expect("parse should succeed");
    let Command::Voice { action } = cli.command else {
        panic!("expected voice command");
    };
    let VoiceAction::Rvc { action } = action else {
        panic!("expected voice rvc command");
    };
    let VoiceRvcAction::Set { name } = action else {
        panic!("expected set action");
    };
    assert_eq!(name, "miku");
}

#[test]
fn voice_rvc_off_command_parses() {
    let cli = Cli::try_parse_from(["kotoba", "voice", "rvc", "off"])
        .expect("parse should succeed");
    let Command::Voice { action } = cli.command else {
        panic!("expected voice command");
    };
    let VoiceAction::Rvc { action } = action else {
        panic!("expected voice rvc command");
    };
    assert!(matches!(action, VoiceRvcAction::Off));
}
```

**Step 3: Run tests to verify parsing**

Run: `cargo test --lib cli::tests`
Expected: PASS for all three new tests

**Step 4: Commit**

```
feat(cli): add voice rvc subcommand enum variants (#N)
```

---

### Task 2: Implement RVC model discovery and fuzzy matching

**Files:**
- Modify: `src/cli/voice.rs`

**Step 1: Add `RvcModelInfo` struct and `list_rvc_models` function**

Add to `src/cli/voice.rs`:

```rust
/// An RVC model entry for display.
#[derive(Debug, Serialize)]
pub struct RvcModelInfo {
    /// Directory name of the model.
    pub name: String,
    /// Whether this model is currently active.
    pub active: bool,
    /// Whether model.pth exists in the directory.
    pub has_pth: bool,
    /// Whether model.index exists in the directory.
    pub has_index: bool,
}

/// Scan the RVC models directory and return all valid models.
pub fn list_rvc_models() -> Result<Vec<RvcModelInfo>> {
    let rvc_dir = crate::paths::models_dir().join("rvc");
    let active_model = crate::app_config::load().rvc.model.clone();

    let mut models: Vec<RvcModelInfo> = Vec::new();

    let entries = match std::fs::read_dir(&rvc_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(models), // no rvc dir yet
    };

    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let dir = entry.path();
        models.push(RvcModelInfo {
            active: name == active_model,
            has_pth: dir.join("model.pth").exists(),
            has_index: dir.join("model.index").exists(),
            name,
        });
    }

    models.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(models)
}

/// Find an RVC model by exact name or case-insensitive substring.
///
/// Returns `Ok(name)` if exactly one model matches, or an error describing
/// zero / ambiguous matches.
pub fn resolve_rvc_model(query: &str) -> Result<String> {
    let models = list_rvc_models()?;
    let valid: Vec<&RvcModelInfo> = models.iter().filter(|m| m.has_pth).collect();

    // Exact match first (case-insensitive)
    if let Some(exact) = valid.iter().find(|m| m.name.eq_ignore_ascii_case(query)) {
        return Ok(exact.name.clone());
    }

    // Substring match (case-insensitive)
    let query_lower = query.to_lowercase();
    let matches: Vec<&&RvcModelInfo> = valid
        .iter()
        .filter(|m| m.name.to_lowercase().contains(&query_lower))
        .collect();

    match matches.len() {
        0 => {
            let available = valid
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            error::VoicevoxSnafu {
                message: format!(
                    "no RVC model matching '{query}' (available: {available})"
                ),
            }
            .fail()
        }
        1 => Ok(matches[0].name.clone()),
        _ => {
            let ambiguous = matches
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            error::VoicevoxSnafu {
                message: format!(
                    "'{query}' matches multiple RVC models: {ambiguous} — be more specific"
                ),
            }
            .fail()
        }
    }
}
```

**Step 2: Write unit tests for resolve logic**

Add to `src/cli/voice.rs` (new `#[cfg(test)] mod tests`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_rvc_models_returns_empty_when_no_dir() {
        // Just ensure it doesn't panic on missing directory
        let result = list_rvc_models();
        assert!(result.is_ok());
    }
}
```

**Step 3: Run tests**

Run: `cargo test --lib cli::voice`
Expected: PASS

**Step 4: Commit**

```
feat(cli): add RVC model discovery and fuzzy matching (#N)
```

---

### Task 3: Implement `list_rvc`, `set_rvc`, `off_rvc` public functions

**Files:**
- Modify: `src/cli/voice.rs`

**Step 1: Add the three command handler functions**

```rust
/// List available RVC models as JSON.
pub fn list_rvc() -> Result<()> {
    let models = list_rvc_models()?;
    let output = serde_json::to_string_pretty(&models).context(error::JsonSnafu)?;
    println!("{output}");
    Ok(())
}

/// Set the active RVC model with fuzzy matching and validation.
pub fn set_rvc(query: &str) -> Result<String> {
    let resolved = resolve_rvc_model(query)?;
    let mut cfg = crate::app_config::load().clone();
    cfg.rvc.model = resolved.clone();
    crate::app_config::save(&cfg).context(error::IoSnafu)?;
    eprintln!("rvc model set to: {resolved}");
    Ok(resolved)
}

/// Disable RVC voice conversion.
pub fn off_rvc() -> Result<()> {
    let mut cfg = crate::app_config::load().clone();
    cfg.rvc.model = String::new();
    crate::app_config::save(&cfg).context(error::IoSnafu)?;
    eprintln!("rvc disabled");
    Ok(())
}
```

**Step 2: Run cargo check**

Run: `cargo check`
Expected: PASS (no compile errors)

**Step 3: Commit**

```
feat(cli): implement voice rvc list/set/off handlers (#N)
```

---

### Task 4: Wire subcommands into main.rs dispatch

**Files:**
- Modify: `src/main.rs`

**Step 1: Add match arms for `VoiceRvcAction`**

In `src/main.rs`, inside the `Command::Voice { action }` match, add after the `VoiceAction::Tone` arm:

```rust
cli::VoiceAction::Rvc { action } => match action {
    cli::VoiceRvcAction::List => {
        cli::voice::list_rvc()?;
    }
    cli::VoiceRvcAction::Set { name } => {
        let resolved = cli::voice::set_rvc(&name)?;
        println!(
            "{}",
            serde_json::json!({"ok": true, "action": "voice_rvc_set", "model": resolved})
        );
    }
    cli::VoiceRvcAction::Off => {
        cli::voice::off_rvc()?;
        println!(
            "{}",
            serde_json::json!({"ok": true, "action": "voice_rvc_off"})
        );
    }
},
```

**Step 2: Run full test suite**

Run: `cargo test`
Expected: PASS

**Step 3: Run clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`
Expected: PASS

**Step 4: Commit**

```
feat(cli): wire voice rvc subcommands into main dispatch (#N)
```

---

### Task 5: Update docs

**Files:**
- Modify: `docs/usage.zh-CN.md`

**Step 1: Add RVC model management section**

After the current section 5 (发音与声音切换), add a subsection for RVC model management:

```markdown
### 5.5 RVC 模型管理

```bash
# 查看所有已下载的 RVC 模型
kotoba voice rvc list

# 设置 RVC 模型（支持模糊匹配）
kotoba voice rvc set miku
kotoba voice rvc set ichika

# 关闭 RVC
kotoba voice rvc off
```

模糊匹配：输入名称的任意子串即可，大小写不敏感。如果匹配到多个模型会提示你输入更精确的名称。
```

**Step 2: Update section 6.3 to reference the new command**

Replace the `kotoba config set rvc.model naruto-rvc-v2` example with:

```markdown
kotoba voice rvc set naruto-rvc-v2
```

**Step 3: Commit**

```
docs: add voice rvc subcommand usage (#N)
```
