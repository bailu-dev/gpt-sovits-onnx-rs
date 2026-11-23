use log::debug;
use std::collections::HashSet;

use crate::preprocessor::zh::jyutping_list::get_jyutping_list;

const INITIALS: &[&str] = &[
    "aa", "aai", "aak", "aap", "aat", "aau", "ai", "au", "ap", "at", "ak", "a", "p", "b", "e",
    "ts", "t", "dz", "d", "kw", "k", "gw", "g", "f", "h", "l", "m", "ng", "n", "s", "y", "w", "c",
    "z", "j", "ong", "on", "ou", "oi", "ok", "o", "uk", "ung", "sp", "spl", "spn", "sil",
];

lazy_static::lazy_static! {
    static ref PUNCTUATION_SET: HashSet<char> = {
        let punctuation = ",.!?;:()[]{}'\"-…";
        punctuation.chars().collect()
    };
}

pub fn g2p(text: &str) -> (Vec<String>, Vec<i32>) {
    let jyutping_list = get_jyutping_list(text);
    debug!("jyutping_list: {:?}", jyutping_list);

    let mut phones = Vec::new();
    let mut word2ph = Vec::new();

    for (character, jyutping) in jyutping_list {
        if character
            .chars()
            .next()
            .is_some_and(|c| PUNCTUATION_SET.contains(&c))
        {
            phones.push(character);
            word2ph.push(1);
        } else if let Some((initial, final_, tone)) = split_jyutping(&jyutping) {
            phones.push(format!("Y{initial}"));
            phones.push(format!("Y{final_}{tone}"));
            word2ph.push(2);
        } else {
            phones.push("UNK".into());
            word2ph.push(1);
        }
    }

    (phones, word2ph)
}

fn split_jyutping(jyutping: &str) -> Option<(&str, &str, &str)> {
    let syllable = jyutping.trim_end_matches(|c: char| c.is_ascii_digit());
    let tone = &jyutping[syllable.len()..];
    // Match in model vocabulary order, so ng and gw precede n and g.
    let initial = INITIALS
        .iter()
        .copied()
        .find(|initial| syllable.starts_with(initial))?;
    let remaining = &syllable[initial.len()..];
    let final_ = if remaining.is_empty() {
        &initial[initial.len() - 1..]
    } else {
        remaining
    };
    Some((initial, final_, tone))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preprocessor::phone_symbol::SYMBOLS;

    #[test]
    fn cantonese_punctuation_and_compound_initials_match_the_model() {
        let (phones, word2ph) = g2p("我,光.");
        assert_eq!(phones, ["Yng", "Yo5", ",", "Ygw", "Yong1", "."]);
        assert_eq!(word2ph, [2, 1, 2, 1]);
        assert!(phones.iter().all(|phone| SYMBOLS.contains_key(phone)));
    }
}
