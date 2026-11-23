// preprocessor/lang.rs
use once_cell::sync::Lazy;
use regex::Regex;

static KANA: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\p{Hiragana}\p{Katakana}]").unwrap());

pub(crate) fn contains_kana(text: &str) -> bool {
    KANA.is_match(text)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lang {
    Zh,
    En,
    Ja,
}

impl Default for Lang {
    fn default() -> Self {
        Lang::Zh
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LangId {
    /// Detect Japanese and English, with Mandarin as the default for Han characters.
    Auto,
    /// Detect Japanese and English, with Cantonese as the default for Han characters.
    AutoYue,
    /// Read Han characters as Japanese, including in text without kana.
    Japanese,
}

impl LangId {
    pub(crate) fn uses_japanese(self, text: &str) -> bool {
        matches!(self, Self::Japanese) || contains_kana(text)
    }
}
