//! UI locale. Scope is **European Latin only** (owner decision 2026-11-01):
//! Italian, English, French, Spanish. No RTL, one always-resident Latin font.
//!
//! The companion's language picker pushes one of these to the Glass via
//! [`crate::messages::SetLocale`]; glassd persists it and re-renders the UI.

use serde::{Deserialize, Serialize};

/// The four supported UI locales. Nothing else is supported by decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
    /// Italiano
    It,
    /// English
    En,
    /// Français
    Fr,
    /// Español
    Es,
}

impl Locale {
    /// Default locale when none has been chosen yet.
    pub const DEFAULT: Locale = Locale::En;

    /// All supported locales, in picker order.
    pub const ALL: [Locale; 4] = [Locale::It, Locale::En, Locale::Fr, Locale::Es];

    /// ISO 639-1 code (`it`, `en`, `fr`, `es`).
    pub fn code(self) -> &'static str {
        match self {
            Locale::It => "it",
            Locale::En => "en",
            Locale::Fr => "fr",
            Locale::Es => "es",
        }
    }

    /// Endonym shown in the language picker.
    pub fn endonym(self) -> &'static str {
        match self {
            Locale::It => "Italiano",
            Locale::En => "English",
            Locale::Fr => "Français",
            Locale::Es => "Español",
        }
    }

    /// Parse an ISO 639-1 code (case-insensitive). Returns `None` for anything
    /// outside the supported set.
    pub fn from_code(code: &str) -> Option<Locale> {
        match code.to_ascii_lowercase().as_str() {
            "it" => Some(Locale::It),
            "en" => Some(Locale::En),
            "fr" => Some(Locale::Fr),
            "es" => Some(Locale::Es),
            _ => None,
        }
    }
}

impl Default for Locale {
    fn default() -> Self {
        Locale::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_roundtrip() {
        for l in Locale::ALL {
            assert_eq!(Locale::from_code(l.code()), Some(l));
            assert_eq!(Locale::from_code(&l.code().to_uppercase()), Some(l));
        }
    }

    #[test]
    fn rejects_unsupported() {
        assert_eq!(Locale::from_code("de"), None);
        assert_eq!(Locale::from_code("ar"), None);
        assert_eq!(Locale::from_code("zh"), None);
    }
}
