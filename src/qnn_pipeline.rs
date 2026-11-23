use crate::vocoder::qnn::{QnnVocoder, report};
use crate::{GSVError, LangId, PostprocessParams, SamplingParams, TTSModel};
use hound::WavSpec;
use ndarray::ArrayD;
use std::{path::Path, sync::mpsc::sync_channel};

impl TTSModel {
    /// Load the QNN generator alongside the ordinary CPU VITS fallback graph.
    pub fn enable_qnn_pipeline(&mut self, models: &Path, libraries: &Path) -> Result<(), GSVError> {
        if self.has_streaming_vits() {
            return Err(GSVError::from(
                "QNN pipeline requires an ordinary VITS graph",
            ));
        }
        if self.reference_paths.vits.is_none() {
            return Err(GSVError::from(
                "HTP VITS requires cached reference features",
            ));
        }
        let vocoder = self
            .sovits
            .as_mut()
            .expect("vocoder is available outside the pipeline");
        vocoder.htp = match QnnVocoder::load(models, libraries, None) {
            Ok(htp) => Some(htp),
            Err(error) => {
                report(log::Level::Warn, format_args!("htp_load_failed {error}"));
                return Err(error);
            }
        };
        Ok(())
    }

    /// Compile a device-specific EPContext and its external binary.
    pub fn compile_qnn_context(
        models: &Path,
        libraries: &Path,
        output: &Path,
    ) -> Result<(), GSVError> {
        QnnVocoder::load(models, libraries, Some(output))?;
        Ok(())
    }

    pub fn has_qnn_pipeline(&self) -> bool {
        self.sovits
            .as_ref()
            .is_some_and(|vocoder| vocoder.htp.is_some())
    }

    /// Run T2S for the next sentence while the vocoder renders the previous one.
    /// Load the generator with [`Self::enable_qnn_pipeline`] before calling this.
    /// Emit each completed HTP window in order after trimming its overlap.
    /// CPU VITS fallback emits a complete sentence. Returning false stops synthesis.
    pub fn synthesize_qnn_pipeline(
        &mut self,
        text: &str,
        sampling: SamplingParams,
        lang: LangId,
        cancelled: impl Fn() -> bool + Sync,
        mut emit: impl FnMut(WavSpec, Vec<f32>) -> bool + Send,
    ) -> Result<(), GSVError> {
        if !self.has_qnn_pipeline() {
            return Err(GSVError::from(
                "QNN synthesis requires enable_qnn_pipeline before inference",
            ));
        }
        let reference = self
            .ref_data
            .as_ref()
            .ok_or_else(|| GSVError::from("Reference data not initialized"))?
            .clone();
        let started = std::time::Instant::now();
        let chunks = self.text_processor.prepare_speech_chunks(text, lang)?;
        let spec = self.output_spec;
        let fragment_interval = PostprocessParams::default().fragment_interval;
        let mut vocoder = self
            .sovits
            .take()
            .expect("vocoder is available outside the pipeline");
        let result = std::thread::scope(|scope| {
            let (sender, receiver) = sync_channel::<(Vec<i64>, ArrayD<i64>, usize)>(1);
            let reference = &reference;
            let cancelled = &cancelled;
            let renderer = &mut vocoder;
            let worker = scope.spawn(move || {
                let mut first_pcm = true;
                for (phones, tokens, pause_samples) in receiver {
                    if cancelled() {
                        break;
                    }
                    let completed = renderer.render_qnn(
                        &phones,
                        &tokens,
                        reference,
                        pause_samples,
                        |samples| {
                            if cancelled() {
                                return false;
                            }
                            if first_pcm {
                                first_pcm = false;
                                report(
                                    log::Level::Info,
                                    format_args!("pipeline_first_pcm_ms={}", started.elapsed().as_millis()),
                                );
                            }
                            emit(spec, samples)
                        },
                    )?;
                    if !completed {
                        break;
                    }
                }
                Ok(())
            });
            let produce = || -> Result<(), GSVError> {
                for (chunk, pause) in chunks {
                    if cancelled() {
                        break;
                    }
                    let tokens = self.synthesize_semantic(
                        &chunk.bert,
                        &chunk.phone_ids,
                        reference,
                        sampling,
                    )?;
                    let pause_samples = pause.samples(spec.sample_rate, fragment_interval);
                    if cancelled()
                        || sender
                            .send((chunk.phone_ids, tokens, pause_samples))
                            .is_err()
                    {
                        break;
                    }
                }
                Ok(())
            };
            let produced = produce();
            drop(sender);
            let rendered = worker
                .join()
                .map_err(|_| GSVError::from("VITS worker panicked"))?;
            rendered.and(produced)
        });
        self.sovits = Some(vocoder);
        result
    }
}
