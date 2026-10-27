/// Names for configurable keys ("Tab", "F12", "Shift+F12", "Ctrl+T", "A", "5"), shared
/// by the Button Mapping capture and every place that checks a binding.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The binding name of a key press, or None for keys that cannot be bound.
pub fn binding_name(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    let (base, shift_matters) = match key.code {
        // Shift is already part of the character itself (a → A, 1 → !)
        KeyCode::Char(' ') => ("Space".to_string(), false),
        KeyCode::Char(c) if !c.is_control() => (c.to_uppercase().collect(), false),
        KeyCode::F(n) => (format!("F{}", n), true),
        KeyCode::Tab => ("Tab".into(), true),
        KeyCode::BackTab => ("Tab".into(), false),
        KeyCode::Enter => ("Enter".into(), true),
        KeyCode::Esc => ("Esc".into(), true),
        KeyCode::Backspace => ("Backspace".into(), true),
        KeyCode::Up => ("Up".into(), true),
        KeyCode::Down => ("Down".into(), true),
        KeyCode::Left => ("Left".into(), true),
        KeyCode::Right => ("Right".into(), true),
        KeyCode::Home => ("Home".into(), true),
        KeyCode::End => ("End".into(), true),
        KeyCode::PageUp => ("PageUp".into(), true),
        KeyCode::PageDown => ("PageDown".into(), true),
        KeyCode::Insert => ("Insert".into(), true),
        KeyCode::Delete => ("Delete".into(), true),
        _ => return None,
    };

    let mut name = String::new();
    if ctrl {
        name.push_str("Ctrl+");
    }
    if alt {
        name.push_str("Alt+");
    }
    if (shift && shift_matters) || key.code == KeyCode::BackTab {
        name.push_str("Shift+");
    }
    name.push_str(&base);
    Some(name)
}

/// True when the key press is the configured binding. "None" / "" never match.
pub fn matches(key: &KeyEvent, binding: &str) -> bool {
    if binding.is_empty() || binding == "None" {
        return false;
    }
    binding_name(key).map_or(false, |name| name.eq_ignore_ascii_case(binding))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn names_keys_like_the_settings_page_shows_them() {
        assert_eq!(binding_name(&key(KeyCode::F(12), KeyModifiers::NONE)).as_deref(), Some("F12"));
        assert_eq!(binding_name(&key(KeyCode::F(12), KeyModifiers::SHIFT)).as_deref(), Some("Shift+F12"));
        assert_eq!(binding_name(&key(KeyCode::Char('t'), KeyModifiers::CONTROL)).as_deref(), Some("Ctrl+T"));
        assert_eq!(binding_name(&key(KeyCode::Char('a'), KeyModifiers::NONE)).as_deref(), Some("A"));
        assert_eq!(binding_name(&key(KeyCode::Char('A'), KeyModifiers::SHIFT)).as_deref(), Some("A"));
        assert_eq!(binding_name(&key(KeyCode::BackTab, KeyModifiers::SHIFT)).as_deref(), Some("Shift+Tab"));
        assert_eq!(binding_name(&key(KeyCode::Tab, KeyModifiers::NONE)).as_deref(), Some("Tab"));
    }

    #[test]
    fn plain_and_shifted_function_keys_differ() {
        let f12 = key(KeyCode::F(12), KeyModifiers::NONE);
        assert!(matches(&f12, "F12"));
        assert!(!matches(&f12, "Shift+F12"));
        assert!(!matches(&f12, "None"));
    }
}
