// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Where authored strategies live: a directory of plain-text files, beside
//! the preferences file and for the same reason — the text form is already
//! the serialization, so there is nothing here for serde to do.
//!
//! A strategy the user wrote and cannot get back is worse than one they
//! never wrote. Until this existed every strategy anyone authored was
//! destroyed on quit, which is why this was pulled ahead of the rule editor
//! (`STRATEGY_DSL.md` Part II, P6a).
//!
//! What this deliberately does *not* do yet is teach the Scenario Sentence
//! to reference a strategy by name and hash — §10's design. That touches
//! the sentence codec and its round-trip law, and belongs with the rest of
//! the provenance story rather than riding along here.

use std::path::{Path, PathBuf};

/// One saved strategy, as it sits on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedStrategy {
    /// The file's stem — what the user picks from, and what they typed.
    pub name: String,
    pub path: PathBuf,
}

/// The directory strategies live in, alongside `prefs.txt`.
pub fn strategies_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("craps-sim").join("strategies"))
}

/// A file name that cannot escape the strategies directory or collide with
/// the shell. Names come from a text field, so this is the only thing
/// between a typed name and the filesystem.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    // A name that survived with nothing but separators in it — "...", "/",
    // "   " — is not a name. Falling back beats writing "---.craps".
    if !trimmed.chars().any(|c| c.is_alphanumeric()) {
        "untitled".to_owned()
    } else {
        trimmed.chars().take(64).collect()
    }
}

/// Every strategy on disk, by name. Unreadable entries are skipped rather
/// than failing the listing — one bad file must not hide the rest.
pub fn list(dir: &Path) -> Vec<SavedStrategy> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<SavedStrategy> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "craps"))
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_stem()?.to_str()?.to_owned();
            Some(SavedStrategy { name, path })
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// Write a strategy, creating the directory if it is not there yet.
pub fn save(dir: &Path, name: &str, source: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.craps", sanitize(name)));
    std::fs::write(&path, source)
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(path)
}

pub fn load(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))
}

/// Remove a saved strategy. Deleting the user's work is the one operation
/// here that cannot be undone, so the caller confirms first.
pub fn delete(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| format!("Could not delete {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("craps-strategies-test-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_strategy_survives_the_round_trip_to_disk() {
        let d = scratch("roundtrip");
        let src = "strategy \"Mine\" language 1\non come-out:\n    bet pass\n";
        let path = save(&d, "Mine", src).unwrap();
        assert_eq!(load(&path).unwrap(), src);
        let all = list(&d);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Mine");
        delete(&all[0].path).unwrap();
        assert!(list(&d).is_empty());
    }

    #[test]
    fn a_typed_name_cannot_escape_the_directory() {
        // The property, not a particular spelling: nothing that survives
        // can name a directory or walk out of one.
        for hostile in [
            "../../etc/passwd",
            "a/b\\c",
            "..",
            "/absolute",
            "with\0null",
            "trailing.",
        ] {
            let out = sanitize(hostile);
            assert!(!out.contains('/'), "{hostile} -> {out}");
            assert!(!out.contains('\\'), "{hostile} -> {out}");
            assert!(!out.contains(".."), "{hostile} -> {out}");
            assert!(!out.is_empty(), "{hostile} -> empty");
            assert_eq!(
                Path::new(&out).components().count(),
                1,
                "{hostile} -> {out} is not a single path component"
            );
        }
        assert_eq!(sanitize("   "), "untitled");
        assert_eq!(sanitize(""), "untitled");
        assert_eq!(sanitize("..."), "untitled");
        assert!(sanitize(&"x".repeat(500)).len() <= 64);
        // The names people actually use survive intact.
        assert_eq!(sanitize("44 Inside, regressed"), "44 Inside- regressed");
        assert_eq!(sanitize("iron-cross_v2"), "iron-cross_v2");
    }

    #[test]
    fn saving_twice_under_one_name_replaces_rather_than_multiplies() {
        let d = scratch("replace");
        save(&d, "S", "one").unwrap();
        save(&d, "S", "two").unwrap();
        let all = list(&d);
        assert_eq!(all.len(), 1);
        assert_eq!(load(&all[0].path).unwrap(), "two");
    }

    #[test]
    fn a_missing_directory_lists_empty_rather_than_failing() {
        assert!(list(&scratch("absent")).is_empty());
    }

    #[test]
    fn one_unreadable_entry_does_not_hide_the_rest() {
        let d = scratch("mixed");
        save(&d, "Good", "strategy \"Good\" language 1\n").unwrap();
        std::fs::write(d.join("notes.txt"), "not a strategy").unwrap();
        let all = list(&d);
        assert_eq!(all.len(), 1, "only .craps files are listed");
        assert_eq!(all[0].name, "Good");
    }
}
