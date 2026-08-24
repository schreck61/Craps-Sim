// Copyright (c) 2026 Jim Schreckengast
// SPDX-License-Identifier: MIT

//! Preferences: a hand-rolled plain-text file, consistent with the no-serde
//! stance. Exactly the keys the plan allows — theme and register — plus the
//! reduced-motion override (an accessibility setting, stored with them).
//! Format: one `key = value` per line; unknown keys are ignored on load
//! and dropped on save (prefs are workstation comfort, never sim inputs).

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Prefs {
    /// Explicit theme choice; `None` follows the OS.
    pub dark: Option<bool>,
    /// Story (false) vs Ledger (true) register.
    pub ledger_register: bool,
    /// In-app reduced-motion override.
    pub reduced_motion: bool,
}

fn prefs_path() -> Option<PathBuf> {
    let base = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.map(|b| b.join("craps-sim").join("prefs.txt"))
}

impl Prefs {
    pub fn load() -> Self {
        let Some(path) = prefs_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Self {
        let mut p = Self::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match (k.trim(), v.trim()) {
                ("theme", "dark") => p.dark = Some(true),
                ("theme", "light") => p.dark = Some(false),
                ("theme", "system") => p.dark = None,
                ("register", "ledger") => p.ledger_register = true,
                ("register", "story") => p.ledger_register = false,
                ("reduced_motion", "on") => p.reduced_motion = true,
                ("reduced_motion", "off") => p.reduced_motion = false,
                _ => {}
            }
        }
        p
    }

    pub fn render(&self) -> String {
        format!(
            "theme = {}\nregister = {}\nreduced_motion = {}\n",
            match self.dark {
                Some(true) => "dark",
                Some(false) => "light",
                None => "system",
            },
            if self.ledger_register {
                "ledger"
            } else {
                "story"
            },
            if self.reduced_motion { "on" } else { "off" },
        )
    }

    pub fn save(&self) {
        let Some(path) = prefs_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, self.render());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for p in [
            Prefs::default(),
            Prefs {
                dark: Some(true),
                ledger_register: true,
                reduced_motion: true,
            },
            Prefs {
                dark: Some(false),
                ledger_register: false,
                reduced_motion: false,
            },
        ] {
            assert_eq!(Prefs::parse(&p.render()), p);
        }
    }

    #[test]
    fn tolerates_junk_and_unknown_keys() {
        let p = Prefs::parse("garbage\nfuture_key = 7\ntheme = dark\n");
        assert_eq!(p.dark, Some(true));
    }
}
