//! Japanese pronunciation using NAIST-JDic full-context labels.
use std::path::PathBuf;

use anyhow::{Context, Result};
use jpreprocess::{DefaultTokenizer, JPreprocess, SystemDictionaryConfig};
use once_cell::sync::Lazy;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use super::seg::replace_consecutive_punctuation;

static TEXT_PARTS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[a-z0-9\p{Han}\p{Hiragana}\p{Katakana}ー々]+|\S").unwrap());

pub(crate) struct Japanese {
    dictionary_dir: PathBuf,
    frontend: Option<JPreprocess<DefaultTokenizer>>,
}

impl Japanese {
    pub(crate) fn new(dictionary_dir: PathBuf) -> Self {
        Self {
            dictionary_dir,
            frontend: None,
        }
    }

    pub(crate) fn load(&mut self) -> Result<()> {
        if self.frontend.is_none() {
            let dictionary = SystemDictionaryConfig::File(self.dictionary_dir.clone())
                .load()
                .with_context(|| {
                    format!(
                        "Cannot load Japanese dictionary at {}. Run scripts/prepare_japanese_dictionary.sh with the model directory.",
                        self.dictionary_dir.display()
                    )
                })?;
            self.frontend = Some(JPreprocess::with_dictionaries(dictionary, None));
        }
        Ok(())
    }

    pub(crate) fn phones(&self, text: &str) -> Result<Vec<String>> {
        let frontend = self.frontend.as_ref().with_context(|| {
            format!(
                "Japanese dictionary is not loaded at {}",
                self.dictionary_dir.display()
            )
        })?;
        let mut phones = Vec::new();
        for part in TEXT_PARTS.find_iter(text).map(|part| part.as_str()) {
            if let Some(punctuation) = match part {
                "。" => Some("."),
                "、" | "·" | ":" | ";" => Some(","),
                "." | "," | "!" | "?" | "…" | "-" => Some(part),
                _ => None,
            } {
                phones.push(punctuation.to_owned());
                continue;
            }
            let labels = frontend.extract_fullcontext(part)?;
            // Follow GPT-SoVITS's Japanese frontend. The model gets interior
            // phones and accent markers, with text punctuation supplying the ends.
            for (index, label) in labels.iter().enumerate() {
                let phone = match label.phoneme.c.as_deref() {
                    None | Some("sil") => continue,
                    Some("pau") => "_",
                    Some("A") => "a",
                    Some("E") => "e",
                    Some("I") => "i",
                    Some("O") => "o",
                    Some("U") => "u",
                    Some(phone) => phone,
                };
                phones.push(phone.to_owned());
                let (Some(mora), Some(next), Some(phrase)) = (
                    label.mora.as_ref(),
                    labels.get(index + 1).and_then(|next| next.mora.as_ref()),
                    label.accent_phrase_curr.as_ref(),
                ) else {
                    continue;
                };
                let position = mora.position_forward;
                if mora.position_backward == 1
                    && next.position_forward == 1
                    && "aeiouNcl".contains(phone)
                {
                    // Upstream's phrase boundary '#' maps to UNK in symbols2.
                    phones.push("UNK".into());
                } else if mora.relative_accent_position == 0
                    && next.position_forward == position.saturating_add(1)
                    && position != phrase.mora_count
                {
                    phones.push("]".into());
                } else if position == 1 && next.position_forward == 2 {
                    phones.push("[".into());
                }
            }
        }
        Ok(phones)
    }
}

pub(crate) fn normalize(text: &str) -> String {
    let text = text.nfkc().collect::<String>().to_lowercase();
    replace_consecutive_punctuation(&text.replace('%', "パーセント"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preprocessor::phoneme_finalize::phones_to_ids;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Golden {
        text: String,
        phones: Vec<String>,
        phone_ids: Vec<i64>,
    }

    #[test]
    fn normalize_preserves_halfwidth_voicing_and_long_vowels() {
        assert_eq!(normalize("ｽｰﾊﾟｰでﾊﾟﾝを買う｡"), "スーパーでパンを買う。");
    }

    #[test]
    #[ignore = "requires NAIST-JDic via GSV_TEST_JAPANESE_DICTIONARY"]
    fn japanese_phones_match_upstream() {
        let path = std::env::var_os("GSV_TEST_JAPANESE_DICTIONARY").unwrap();
        let mut japanese = Japanese::new(PathBuf::from(path));
        japanese.load().unwrap();
        // GPT-SoVITS 04a44eb with pyopenjtalk 0.4.1 and its base dictionary.
        let fixtures: Vec<Golden> = serde_json::from_str(include_str!(
            "../../resource/japanese_preprocess_golden.json"
        ))
        .unwrap();
        for fixture in fixtures {
            let actual = japanese.phones(&normalize(&fixture.text)).unwrap();
            assert_eq!(actual, fixture.phones, "{}", fixture.text);
            assert_eq!(
                phones_to_ids(&actual, &fixture.text),
                fixture.phone_ids,
                "{}",
                fixture.text
            );
        }
    }
}
