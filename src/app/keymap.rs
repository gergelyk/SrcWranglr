//! The global keyboard shortcuts, and `~/.config/srcwranglr/shortcuts.toml`,
//! where you rebind them (read at startup):
//!
//! ```toml
//! back = "Alt+Left"
//! open = "Cmd+O"            # Cmd is Ctrl off macOS
//! findings = "Cmd+Shift+R"
//! ```
//!
//! A shortcut is modifiers (`Cmd`, `Ctrl`, `Alt`, `Shift`) and a key, joined
//! by `+`. Names that don't parse, or aren't listed here, are ignored.

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use egui::{Key, KeyboardShortcut, Modifiers};
use toml_edit::DocumentMut;

use super::dangerous_lists::config_dir;

pub const SHORTCUTS_FILE: &str = "shortcuts.toml";
/// A shortcuts file larger than this isn't read.
const MAX_FILE_BYTES: u64 = 64 * 1024;

const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);

static USER: LazyLock<HashMap<String, KeyboardShortcut>> = LazyLock::new(|| {
    config_dir()
        .map(|dir| load(&dir.join(SHORTCUTS_FILE)))
        .unwrap_or_default()
});

fn load(path: &Path) -> HashMap<String, KeyboardShortcut> {
    let Ok(meta) = std::fs::metadata(path) else {
        return HashMap::new();
    };
    if meta.len() > MAX_FILE_BYTES {
        return HashMap::new();
    }
    std::fs::read_to_string(path)
        .map(|text| parse(&text))
        .unwrap_or_default()
}

fn parse(text: &str) -> HashMap<String, KeyboardShortcut> {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return HashMap::new();
    };
    doc.iter()
        .filter_map(|(name, item)| {
            let shortcut = parse_shortcut(item.as_str()?)?;
            Some((name.to_string(), shortcut))
        })
        .collect()
}

/// `Cmd+Shift+R`, `Alt+Left`, … Case-insensitive.
pub fn parse_shortcut(text: &str) -> Option<KeyboardShortcut> {
    let mut modifiers = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "cmd" | "command" => modifiers = modifiers.plus(Modifiers::COMMAND),
            "ctrl" | "control" => modifiers = modifiers.plus(Modifiers::CTRL),
            "alt" | "option" => modifiers = modifiers.plus(Modifiers::ALT),
            "shift" => modifiers = modifiers.plus(Modifiers::SHIFT),
            _ if key.is_some() => return None,
            "left" => key = Some(Key::ArrowLeft),
            "right" => key = Some(Key::ArrowRight),
            "up" => key = Some(Key::ArrowUp),
            "down" => key = Some(Key::ArrowDown),
            _ => key = Some(Key::from_name(part)?),
        }
    }
    key.map(|key| KeyboardShortcut::new(modifiers, key))
}

fn bound(name: &str, default: KeyboardShortcut) -> KeyboardShortcut {
    USER.get(name).copied().unwrap_or(default)
}

macro_rules! shortcuts {
    ($($const_name:ident, $name:literal, $mods:expr, $key:expr;)*) => {
        $(
            pub static $const_name: LazyLock<KeyboardShortcut> =
                LazyLock::new(|| bound($name, KeyboardShortcut::new($mods, $key)));
        )*
    };
}

shortcuts! {
    OPEN_SHORTCUT, "open", CMD, Key::O;
    ENTRY_POINT_SHORTCUT, "entry_point", CMD, Key::E;
    EDITOR_SHORTCUT, "editor", CMD_SHIFT, Key::E;
    BOOKMARK_SHORTCUT, "bookmark", CMD, Key::D;
    NOTE_SHORTCUT, "note", CMD_SHIFT, Key::N;
    NOTES_PANEL_SHORTCUT, "notes_panel", CMD, Key::B;
    FINDINGS_SHORTCUT, "findings", CMD_SHIFT, Key::R;
    TRIAGE_SHORTCUT, "triage", CMD_SHIFT, Key::T;
    LAYOUT_SHORTCUT, "layout", CMD_SHIFT, Key::L;
    SETTINGS_SHORTCUT, "settings", CMD, Key::Comma;
    SAVE_SESSION_SHORTCUT, "save_session", CMD, Key::S;
    JOURNAL_SHORTCUT, "journal", CMD, Key::J;
    BACK_SHORTCUT, "back", Modifiers::ALT, Key::ArrowLeft;
    OPEN_SYMBOL_SHORTCUT, "go_to_symbol", CMD, Key::P;
    OPEN_COMMAND_SHORTCUT, "command_palette", CMD_SHIFT, Key::P;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_with_modifiers_and_key_names() {
        assert_eq!(
            parse_shortcut("alt+Left"),
            Some(KeyboardShortcut::new(Modifiers::ALT, Key::ArrowLeft))
        );
        assert_eq!(
            parse_shortcut("Cmd + Shift + R"),
            Some(KeyboardShortcut::new(CMD_SHIFT, Key::R))
        );
        assert_eq!(parse_shortcut("Alt+"), None);
        assert_eq!(parse_shortcut("A+B"), None);
        assert_eq!(parse_shortcut("Nonsense"), None);
    }

    #[test]
    fn a_file_rebinds_only_the_names_it_gives_validly() {
        let map = parse("back = \"Ctrl+Left\"\nopen = 3\nfindings = \"???\"\n");
        assert_eq!(map.len(), 1);
        assert_eq!(
            map["back"],
            KeyboardShortcut::new(Modifiers::CTRL, Key::ArrowLeft)
        );
    }
}
