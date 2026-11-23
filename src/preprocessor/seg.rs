// Sentence boundaries for inference and punctuation cleanup for phonemization.

use regex::Regex;

pub const SPLITS: &[char] = &[
    '，', '。', '？', '！', ',', '.', '?', '!', '~', ':', '：', ';', '；', '—', '…',
];

/// Collapse consecutive punctuation (Python `replace_consecutive_punctuation`).
pub fn replace_consecutive_punctuation(text: &str) -> String {
    let punct: String = SPLITS
        .iter()
        .chain(&['!', '?', '…', ',', '.', '-'])
        .map(|c| regex::escape(&c.to_string()))
        .collect();
    let pattern = format!(r"([{punct}])([{punct}])+");
    let re = Regex::new(&pattern).unwrap();
    re.replace_all(text, "$1").into_owned()
}

const MAX_CHARS: usize = 510;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Pause {
    Clause,
    Sentence,
    Paragraph,
}

impl Pause {
    pub(crate) fn samples(self, sample_rate: u32, fragment_interval: f32) -> usize {
        let scale = match self {
            Self::Clause => 0.6,
            Self::Sentence => 1.0,
            Self::Paragraph => 1.5,
        };
        (sample_rate as f32 * fragment_interval.max(0.0) * scale) as usize
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SpeechChunk {
    pub(crate) text: String,
    pub(crate) pause_after: Pause,
}

fn punctuation_pause(c: char) -> Option<Pause> {
    match c {
        '：' | ':' | '；' | ';' => Some(Pause::Clause),
        '。' | '！' | '？' | '．' | '.' | '!' | '?' | '…' => Some(Pause::Sentence),
        _ => None,
    }
}

fn closer(c: char) -> bool {
    matches!(
        c,
        '”' | '’' | '"' | '\'' | '」' | '』' | '》' | '）' | ')' | '】' | ']'
    )
}

fn ascii_word(chars: impl Iterator<Item = char>) -> String {
    chars.take_while(char::is_ascii_alphabetic).collect()
}

fn decimal_digit(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '０'..='９')
}

fn between_digits(chars: &[char], index: usize) -> bool {
    index > 0
        && decimal_digit(chars[index - 1])
        && chars.get(index + 1).copied().is_some_and(decimal_digit)
}

fn internal_period(chars: &[char], index: usize, following: &[char]) -> bool {
    if chars.get(index + 1).is_some_and(|&c| decimal_digit(c))
        && (index == 0
            || !chars[index - 1].is_ascii_alphabetic()
                && !matches!(chars[index - 1], '。' | '.' | '．'))
    {
        return true;
    }
    let Some(&previous) = chars[..index].last() else {
        return false;
    };
    let Some(&next) = following.first() else {
        return false;
    };
    if chars.get(index + 1).is_some_and(|&immediate| {
        previous.is_ascii_alphanumeric() && immediate.is_ascii_lowercase()
    }) {
        return true;
    }

    let word: String = ascii_word(chars[..index].iter().rev().copied())
        .chars()
        .rev()
        .collect();
    let abbreviation = word.to_ascii_lowercase();
    if matches!(
        abbreviation.as_str(),
        "mr" | "mrs" | "ms" | "dr" | "prof" | "sr" | "jr" | "st" | "vs" | "no" | "fig"
    ) {
        return next.is_ascii_alphanumeric();
    }

    let dotted = word.len() == 1
        && index >= 3
        && chars[index - 2] == '.'
        && chars[index - 3].is_ascii_alphabetic();
    if dotted {
        if chars.get(index + 1).is_some_and(char::is_ascii_alphabetic) || next.is_ascii_lowercase()
        {
            return true;
        }
        return matches!(
            (
                chars[index - 3].to_ascii_lowercase(),
                previous.to_ascii_lowercase()
            ),
            ('e', 'g') | ('i', 'e')
        ) && next.is_ascii_alphanumeric();
    }
    word.len() == 1 && previous.is_ascii_uppercase() && next.is_ascii_uppercase()
}

fn inside_url(chars: &[char], index: usize) -> bool {
    if !chars.get(index + 1).is_some_and(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '/' | '%' | '#' | '_' | '~' | '@' | '=')
    }) {
        return false;
    }
    let start = chars[..index]
        .iter()
        .rposition(|c| !c.is_ascii() || c.is_whitespace() || matches!(c, '(' | '[' | '"' | '<'))
        .map_or(0, |position| position + 1);
    let prefix = chars[start..index]
        .iter()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    prefix.starts_with("https://")
        || prefix.starts_with("http://")
        || prefix.starts_with("www.")
        || (prefix == "www" && chars[index] == '.')
        || (matches!(prefix.as_str(), "http" | "https")
            && chars[index..].starts_with(&[':', '/', '/']))
}

fn spoken_text(chars: &[char]) -> Option<String> {
    chars
        .iter()
        .any(|c| c.is_alphanumeric() || matches!(c, '￥' | '^'))
        .then(|| chars.iter().collect::<String>().trim().to_owned())
}

fn split_sentence_chunks(line: &str) -> Vec<SpeechChunk> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let (mut start, mut index, mut parentheses) = (0, 0, 0usize);
    while index < chars.len() {
        let c = chars[index];
        match c {
            '(' | '（' | '[' | '【' => parentheses += 1,
            ')' | '）' | ']' | '】' => parentheses = parentheses.saturating_sub(1),
            _ => {}
        }
        let Some(mut pause_after) = punctuation_pause(c) else {
            index += 1;
            continue;
        };
        if parentheses > 0
            || (matches!(c, ':' | '：') && between_digits(&chars, index))
            || (c.is_ascii() && inside_url(&chars, index))
        {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while let Some(next_pause) = chars.get(end).copied().and_then(punctuation_pause) {
            pause_after = pause_after.max(next_pause);
            end += 1;
        }
        let terminal_end = end;
        while end < chars.len() && closer(chars[end]) {
            end += 1;
        }
        let following_start = chars[end..]
            .iter()
            .position(|c| !c.is_whitespace())
            .map_or(chars.len(), |position| end + position);
        let following = &chars[following_start..];
        let next = following.first().copied();
        let internal = (matches!(c, '.' | '．')
            && terminal_end == index + 1
            && internal_period(&chars, index, following))
            || ((c == '…' || (c == '.' && terminal_end - index >= 3))
                && next.is_some_and(|next| next.is_alphabetic() && !next.is_uppercase()))
            || (end > terminal_end
                && next.is_some_and(|next| {
                    matches!(next, ',' | '，' | ';' | '；' | '—') || next.is_ascii_lowercase()
                }));
        if !internal && chars[start..index].iter().any(|c| c.is_alphanumeric()) {
            let text = chars[start..end]
                .iter()
                .collect::<String>()
                .trim()
                .to_owned();
            out.push(SpeechChunk { text, pause_after });
            start = end;
        }
        index = if internal { index + 1 } else { end };
    }
    if let Some(text) = spoken_text(&chars[start..]) {
        out.push(SpeechChunk {
            text,
            pause_after: Pause::Sentence,
        });
    }
    out
}

fn bounded_chunks(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut remaining = chars.as_slice();
    let mut chunks = Vec::new();
    while remaining.len() > MAX_CHARS {
        // Reserve one character for punctuation on a forced break.
        let cap = MAX_CHARS - 1;
        let boundary = (cap / 2..cap)
            .rev()
            .find(|&i| {
                matches!(remaining[i], ',' | '，' | ';' | '；' | ':' | '：')
                    && !between_digits(remaining, i)
                    && !inside_url(remaining, i)
            })
            .or_else(|| (cap / 2..cap).rev().find(|&i| remaining[i].is_whitespace()))
            .map_or(cap, |position| position + 1);
        chunks.extend(spoken_text(&remaining[..boundary]));
        remaining = &remaining[boundary..];
    }
    chunks.extend(spoken_text(remaining));
    chunks
}

/// Keep spoken boundaries before punctuation and language normalization.
pub(crate) fn pre_seg_text(text: &str, lang_en: bool) -> Vec<SpeechChunk> {
    let terminator = if lang_en { '.' } else { '。' };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut chunks: Vec<SpeechChunk> = Vec::new();
    for line in text.trim().lines() {
        if let Some(previous) = chunks.last_mut() {
            let line_break = if line.trim().is_empty() {
                Pause::Paragraph
            } else {
                Pause::Sentence
            };
            previous.pause_after = previous.pause_after.max(line_break);
        }
        for sentence in split_sentence_chunks(line) {
            let mut parts = bounded_chunks(&sentence.text).into_iter().peekable();
            while let Some(mut chunk) = parts.next() {
                let pause_after = if parts.peek().is_some() {
                    Pause::Sentence
                } else {
                    sentence.pause_after
                };
                if !chunk.chars().rev().find(|&c| !closer(c)).is_some_and(|c| {
                    punctuation_pause(c) == Some(Pause::Sentence)
                        || pause_after == Pause::Clause && punctuation_pause(c).is_some()
                }) {
                    chunk.push(terminator);
                }
                if chunks.is_empty()
                    && chunk
                        .chars()
                        .take_while(|c| !SPLITS.contains(c) && !matches!(c, '￥' | '^'))
                        .count()
                        < 4
                    && !chunk.chars().next().is_some_and(|c| SPLITS.contains(&c))
                    && chunk.chars().count() < MAX_CHARS
                {
                    chunk.insert(0, terminator);
                }
                chunks.push(SpeechChunk {
                    text: chunk,
                    pause_after,
                });
            }
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(chunks: Vec<SpeechChunk>) -> Vec<String> {
        chunks.into_iter().map(|chunk| chunk.text).collect()
    }

    #[test]
    fn test_collapse_punctuation() {
        assert_eq!(
            replace_consecutive_punctuation("你好，，世界"),
            "你好，世界"
        );
    }

    #[test]
    fn test_split_sentence_chunks_ignores_decimal() {
        assert_eq!(
            texts(split_sentence_chunks("价格是99.5元。")),
            vec!["价格是99.5元。"]
        );
    }

    #[test]
    fn test_pre_seg_keeps_short_prefix_with_text() {
        assert_eq!(texts(pre_seg_text("嗯", false)), vec!["。嗯。"]);
    }

    #[test]
    fn test_pre_seg_splits_mixed_tail() {
        let chunks = pre_seg_text(
            "你好啊,这是一个测试.吃葡萄不吐葡萄皮,不吃葡萄倒吐葡萄皮.This demo is only for test  usage. If you find any 问题, 请修复它.",
            false,
        );
        assert!(
            chunks
                .iter()
                .any(|c| c.text.contains("This demo is only for test"))
        );
        assert!(
            chunks
                .iter()
                .any(|c| c.text.contains("If you find any 问题"))
        );
    }

    #[test]
    fn english_abbreviations_decimals_and_urls_stay_inside_the_sentence() {
        assert_eq!(
            texts(split_sentence_chunks(
                "Dr. J. Smith paid 3.14 in the U.S. last week. Try e.g. https://example.com?q=1. Next sentence."
            )),
            [
                "Dr. J. Smith paid 3.14 in the U.S. last week.",
                "Try e.g. https://example.com?q=1.",
                "Next sentence."
            ]
        );
        assert_eq!(
            texts(split_sentence_chunks("We met at 9 a.m. Then we left.")),
            ["We met at 9 a.m.", "Then we left."]
        );
    }

    #[test]
    fn keeps_quoted_sentence_endings_and_inline_ellipses() {
        assert_eq!(
            texts(pre_seg_text(
                "他说：“价格是99.5元，你要买吗？”\n我想……先等等！\n\nShe asked “Why?” quietly. “Ready?!” Then we left.",
                false
            )),
            [
                "。他说：",
                "“价格是99.5元，你要买吗？”",
                "我想……先等等！",
                "She asked “Why?” quietly.",
                "“Ready?!”",
                "Then we left."
            ]
        );
    }

    #[test]
    fn clause_pauses_preserve_times_ratios_and_inline_urls() {
        let chunks = pre_seg_text(
            "第二处是语音检测这套：基线慢慢平滑；继续。现在是12：30，比例16:9。进https://example.com:443/a?q=1。Next: done.",
            false,
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.pause_after)
                .collect::<Vec<_>>(),
            [
                Pause::Clause,
                Pause::Clause,
                Pause::Sentence,
                Pause::Sentence,
                Pause::Sentence,
                Pause::Clause,
                Pause::Sentence
            ]
        );
        assert_eq!(
            texts(chunks),
            [
                "第二处是语音检测这套：",
                "基线慢慢平滑；",
                "继续。",
                "现在是12：30，比例16:9。",
                "进https://example.com:443/a?q=1。",
                "Next:",
                "done.",
            ]
        );
    }

    #[test]
    fn line_breaks_and_punctuation_share_one_pause() {
        let chunks = pre_seg_text(
            "检测结果如下：\r\n基线平滑。\n \t\r\n双阈值滞回\r结束。",
            false,
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.pause_after.samples(32000, 0.3))
                .collect::<Vec<_>>(),
            [9600, 14400, 9600, 9600]
        );
        assert_eq!(
            texts(chunks),
            ["检测结果如下：", "基线平滑。", "双阈值滞回。", "结束。"]
        );
        assert_eq!(Pause::Clause.samples(32000, 0.3), 5760);
        assert_eq!(Pause::Paragraph.samples(32000, 0.0), 0);
        assert_eq!(
            texts(pre_seg_text(
                "This sentence\ncontinues on a new line.",
                true
            )),
            ["This sentence.", "continues on a new line."]
        );
    }

    #[test]
    fn splits_long_sentences_at_clauses_without_discarding_punctuation() {
        let left = "甲".repeat(300);
        let right = "乙".repeat(260);
        assert_eq!(
            texts(pre_seg_text(&format!("{left}，{right}。"), false)),
            [format!("{left}，。"), format!("{right}。")]
        );
    }
}
