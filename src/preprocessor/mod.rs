// preprocessor/mod.rs — TextProcessor orchestration
use anyhow::Result;
use log::{debug, warn};
use ndarray::Array2;

pub mod bert;
pub mod clean;
pub mod dict;
pub mod en;
pub mod g2p;
mod ja;
pub mod lang;
pub mod lang_segment;
pub mod num;
pub mod phone_symbol;
pub mod phoneme_finalize;
pub mod seg;
pub mod text_normalize;
pub mod tokenize;
pub mod utils;
pub mod zh;

pub use lang::{Lang, LangId};
pub use text_normalize::text_normalize;

use crate::preprocessor::{
    bert::BertModel,
    clean::{CleanedInput, infer_lang_en, normalize_input, silence_tags_for_chunk},
    en::g2p_en::G2pEn,
    g2p::{G2pDeps, g2p_spans},
    ja::Japanese,
    lang::contains_kana,
    phoneme_finalize::{ChunkInput, ChunkOutput, needs_short_retry},
    seg::{Pause, pre_seg_text},
    zh::g2pw::G2PW,
};
use jieba_rs::Jieba;
use std::path::PathBuf;
use std::sync::Arc;

pub struct TextProcessor {
    pub jieba: Arc<Jieba>,
    pub g2pw: G2PW,
    pub g2p_en: G2pEn,
    pub bert: BertModel,
    japanese: Japanese,
}

impl TextProcessor {
    pub fn new(g2pw: G2PW, g2p_en: G2pEn, bert: BertModel) -> Result<Self> {
        Ok(Self {
            jieba: Arc::new(Jieba::new()),
            g2pw,
            g2p_en,
            bert,
            japanese: Japanese::new(PathBuf::from("g2p_ja")),
        })
    }

    pub(crate) fn with_japanese_dictionary(mut self, path: PathBuf) -> Self {
        self.japanese = Japanese::new(path);
        self
    }

    pub(crate) fn preload_japanese(&mut self, text: &str, lang: LangId) -> Result<()> {
        if lang.uses_japanese(text) {
            self.japanese.load()?;
        }
        Ok(())
    }

    pub fn get_phone_and_bert(
        &mut self,
        text: &str,
        lang_id: LangId,
    ) -> Result<Vec<(String, Vec<i64>, Array2<f32>)>> {
        Ok(self
            .prepare_speech_chunks(text, lang_id)?
            .into_iter()
            .map(|(output, _)| (output.norm_text, output.phone_ids, output.bert))
            .collect())
    }

    pub(crate) fn prepare_speech_chunks(
        &mut self,
        text: &str,
        lang_id: LangId,
    ) -> Result<Vec<(ChunkOutput, Pause)>> {
        self.preload_japanese(text, lang_id)?;
        let chunks = pre_seg_text(text, infer_lang_en(text));
        let mut result = Vec::with_capacity(chunks.len());

        for chunk in chunks {
            debug!("Processing chunk: {}", chunk.text);
            let cleaned = normalize_input(&chunk.text)?;
            match self.process_chunk(&cleaned, &cleaned.text, lang_id, chunk.line_has_kana, false) {
                Ok(Some(output)) => result.push((output, chunk.pause_after)),
                Ok(None) => {}
                Err(e) if chunk.line_has_kana || matches!(lang_id, LangId::Japanese) => {
                    return Err(e);
                }
                Err(e) => warn!("Failed to process chunk '{}': {}", chunk.text, e),
            }
        }

        debug!("RESULT (total sentences: {})", result.len());
        if result.is_empty() {
            return Err(anyhow::anyhow!(
                "No phonemes or BERT features could be generated for the text: {}",
                text
            ));
        }
        Ok(result)
    }

    /// Phonemize a single utterance without `pre_seg_text` (matches Python
    /// `segment_and_extract_feature_for_text` used for reference prompt text).
    pub fn get_phone_and_bert_whole(
        &mut self,
        text: &str,
        lang_id: LangId,
    ) -> Result<(String, Vec<i64>, Array2<f32>)> {
        self.preload_japanese(text, lang_id)?;
        self.get_phone_and_bert_for_line(text, lang_id, false)
    }

    pub(crate) fn get_phone_and_bert_for_line(
        &mut self,
        text: &str,
        lang_id: LangId,
        line_has_kana: bool,
    ) -> Result<(String, Vec<i64>, Array2<f32>)> {
        let cleaned = normalize_input(text)?;
        match self.process_chunk(&cleaned, &cleaned.text, lang_id, line_has_kana, false)? {
            Some(output) => Ok((output.norm_text, output.phone_ids, output.bert)),
            None => Err(anyhow::anyhow!(
                "No phonemes or BERT features could be generated for the text: {}",
                text
            )),
        }
    }

    fn process_chunk(
        &mut self,
        cleaned: &CleanedInput,
        chunk: &str,
        lang_id: LangId,
        line_has_kana: bool,
        is_final: bool,
    ) -> Result<Option<ChunkOutput>> {
        let japanese = line_has_kana || matches!(lang_id, LangId::Japanese);
        let has_kana = contains_kana(chunk);
        let spans = if !japanese && has_kana && chunk.contains(['\n', '\r']) {
            chunk
                .split(['\n', '\r'])
                .flat_map(|line| lang_segment::get_spans(line, &self.jieba, contains_kana(line)))
                .collect()
        } else {
            lang_segment::get_spans(chunk, &self.jieba, japanese || has_kana)
        };
        if spans.is_empty() {
            return Ok(None);
        }

        let silence_tags = silence_tags_for_chunk(&cleaned.silence_markers, &cleaned.text, chunk);

        let mut deps = G2pDeps {
            g2pw: &mut self.g2pw,
            g2p_en: &mut self.g2p_en,
            jieba: &self.jieba,
            japanese: &self.japanese,
        };
        let span_results = g2p_spans(&spans, lang_id, &mut deps)?;
        if span_results.is_empty() {
            return Ok(None);
        }

        let output = phoneme_finalize::finalize_chunk(
            &mut self.bert,
            ChunkInput {
                spans: span_results,
                silence_tags,
            },
        )?;

        if needs_short_retry(output.phone_ids.len(), is_final) {
            let retry_chunk = format!(".{}", chunk);
            return self.process_chunk(cleaned, &retry_chunk, lang_id, line_has_kana, true);
        }

        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn mixed_request_requires_the_japanese_dictionary_before_phonemizing() {
        let path = PathBuf::from("missing-test-japanese-dictionary/g2p_ja");
        let mut processor = TextProcessor::new(
            G2PW::new(None::<&Path>).unwrap(),
            G2pEn::new(None::<&Path>).unwrap(),
            BertModel::new(None::<&Path>).unwrap(),
        )
        .unwrap()
        .with_japanese_dictionary(path.clone());
        assert!(processor.get_phone_and_bert("入口。", LangId::Auto).is_ok());
        for (text, language) in [
            ("入口。\nこんにちは。", LangId::Auto),
            ("入口。", LangId::Japanese),
        ] {
            let error = processor.get_phone_and_bert(text, language).unwrap_err();
            assert!(error.to_string().contains(&path.display().to_string()));
        }
    }
}
