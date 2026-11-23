# GPT-SoVITS ONNX RS

[简体中文](Readme.md) · **English**

GPT-SoVITS ONNX RS is an optimized inference library for
[GPT-SoVITS](https://github.com/RVC-Boss/GPT-SoVITS). It provides ONNX export tools
and a Rust runtime built on ONNX Runtime for x86-64 and arm64 CPUs. In v2Pro
benchmarks, the library streamed speech within each sentence at about **3.7× real time**
on a Galaxy S25 Ultra and **9.3× real time** on a Mac M4 Pro.

Given a reference WAV, its exact transcript, and text to read, the library
generates 32 kHz mono audio in the reference voice. Rust apps can receive PCM
in chunks or as a complete utterance. The CLI example writes a WAV file using
the same models.

## How it speeds up inference

- **ONNX graphs designed for CPU inference.** Separate graphs handle the first
  T2S step and subsequent tokens. They expose the KV cache so the decoder can
  reuse its buffers. The export tools offer per-model quantization for BERT,
  G2PW, and T2S, and apply CPU graph fusions at export time. The `mobile`
  profile can use INT4 weights for the per-token projections.
  [Export and quantization settings (Chinese)](scripts/README.md#android-cpu-低延迟导出)
- **Reusable reference features.** A separate VITS graph extracts reference
  conditioning. Utterances in the same voice can share the conditioning
  vector and semantic prompt. Hosts can save and restore these features across
  model instances.
- **Concurrent decoding and waveform generation within a sentence.** The
  streaming VITS graph takes a frame offset and renders the current chunk and
  the context needed to join it to adjacent chunks while T2S continues decoding.
  Adaptive streaming uses recent synthesis times and estimated playback headroom
  to choose the next chunk size.

## CPU benchmarks

Both devices used the v2Pro streaming ONNX models and the same 25 texts. Runs
were measured after warm-up, with the first block starting at 8 semantic tokens.

| Device | Mean time to first PCM | Mean RTF |
| --- | ---: | ---: |
| Mac M4 Pro | 175 ms | 0.108 |
| Galaxy S25 Ultra | 386 ms | 0.271 |

RTF is synthesis time divided by the duration of the generated audio. Lower is
faster. Simulated playback ran without underruns on either device. In a separate
multi-sentence test, the S25 Ultra generated 72 seconds of speech at an RTF of
0.219. Tests with individual sentences over 100 seconds sometimes repeated or
skipped words, so split long passages into sentences.
[Benchmark setup, full results, and historical x86-64 data](doc/perf_record.md)

## Supported features

| Feature | Support |
| --- | --- |
| Models | Whole-utterance synthesis with v2, v2Pro, and v2ProPlus. Within-sentence streaming with v2Pro has been tested on macOS and Android CPUs. |
| Languages | Mandarin, English, and mixed Mandarin and English. For Cantonese, use `--lang yue` in the CLI or `LangId::AutoYue` in Rust. |
| Output | Chunked or whole-utterance PCM through the Rust API. The CLI writes WAV files. |
| Platforms | x86-64 and arm64 CPUs. The Android example targets `arm64-v8a`. |
| Reference voices | Change the reference WAV and transcript at runtime. Pro models use `sv.onnx` to extract speaker features. |

## Quick start

The [v2Pro model repository](https://huggingface.co/mikv39/gpt-sovits-onnx-custom)
has separate `quality/` and `mobile/` bundles for desktop and Android. This
example uses `quality` to generate a WAV file on macOS. You need stable Rust,
CMake, Xcode Command Line Tools, and the Hugging Face CLI. The example runs
the CLI through `uvx`. You can also use an installed `hf` command.

```bash
MODEL_ROOT="$PWD/models/gpt-sovits"
uvx --from huggingface_hub hf download mikv39/gpt-sovits-onnx-custom \
  --include 'quality/**' --local-dir "$MODEL_ROOT"
MODEL_DIR="$MODEL_ROOT/quality"
```

Use a mono PCM16 reference WAV, ideally 3–10 seconds long, and pass its exact
transcript to `--ref-text`.

```bash
cargo run --release --example gpt_sovits_demo -- \
  --model-path "$MODEL_DIR" --stream \
  --ref-audio /path/to/voice.wav --ref-text '<exact reference transcript>' \
  --text 'When the rain stops, we will follow the river to the lighthouse.' \
  --output speech.wav
```

- **Generate a complete utterance** using `vits_non_streaming.onnx` from the same bundle.
  [Whole-utterance setup](https://huggingface.co/mikv39/gpt-sovits-onnx-custom#run)
- **Run on Android** with the [demo app or CLI](#build-and-run-on-android).
- **Export your own weights or quantized models** with the [export guide (Chinese)](scripts/README.md#从权重导出).

## Build and run on Android

The [Android TTS Demo](https://github.com/bailu-obj/gpt-sovits-android-tts/blob/main/README.en.md) is a
Kotlin and Compose Material 3 app for the v2Pro `mobile/` bundle. Enter text,
switch reference voices, compare streaming and whole-utterance inference times,
or set it as the Android system TTS engine. Its README covers building the APK
and installing the models.

### Run the CLI on a device

You need stable Rust, CMake, native build tools, the Android NDK, `curl`, `unzip`,
and `adb`. The build host can be macOS or x86-64 Linux. The target is
`arm64-v8a` with API level 28. The commands download the `mobile/` bundle for
Android CPUs. To build a streaming bundle from your own weights, see the
[export guide (Chinese)](scripts/README.md#从权重导出).

```bash
MODEL_ROOT="$PWD/models/gpt-sovits"
uvx --from huggingface_hub hf download mikv39/gpt-sovits-onnx-custom \
  --include 'mobile/**' --local-dir "$MODEL_ROOT"
MODEL_DIR="$MODEL_ROOT/mobile"

rustup target add aarch64-linux-android
export ANDROID_NDK_ROOT=/path/to/android-ndk
./build_for_android.sh

adb devices
DEVICE_DIR=$(adb shell mktemp -d /data/local/tmp/gpt-sovits.XXXXXX)
adb push target/android/gpt_sovits_demo "$DEVICE_DIR/gpt_sovits_demo"
adb push target/android/lib "$DEVICE_DIR/lib"
adb push "$MODEL_DIR" "$DEVICE_DIR/models"
adb push /path/to/voice.wav "$DEVICE_DIR/ref.wav"
adb shell "chmod 700 '$DEVICE_DIR/gpt_sovits_demo'"
adb shell "LD_LIBRARY_PATH='$DEVICE_DIR/lib' '$DEVICE_DIR/gpt_sovits_demo' \
  --ort-library '$DEVICE_DIR/lib/libonnxruntime.so' --model-path '$DEVICE_DIR/models' \
  --ref-audio '$DEVICE_DIR/ref.wav' --ref-text '<exact reference transcript>' \
  --stream --text 'This speech was generated on an Android device.' --output '$DEVICE_DIR/speech.wav'"
adb pull "$DEVICE_DIR/speech.wav" target/android/speech.wav
```

The build script provides the CLI, the full ONNX Runtime 1.30.0 library,
`libc++_shared.so`, and a whole-utterance JNI example. Device tests used NDK r29
(29.0.14206865). Use `--output-dir` to set the output location and
`--archive-dir` to reuse downloaded archives.
[Android app integration (Chinese)](scripts/README.md#android-app-接入)

## Rust API

Open a model bundle with `TTSModel::from_bundle` and prepare the reference
audio with `process_reference_sync`. Load the bundle's sampling settings
with `InferParams::from_bundle`.

| Output | VITS graph | API |
| --- | --- | --- |
| Streaming | Streaming VITS with `stream_start_frame` | `synthesize_token_chunks_sync` delivers `WavSpec` and PCM to a callback |
| Whole utterance | Standard VITS | `synthesize_sync` returns `(WavSpec, Vec<f32>)` |
| Qualcomm HTP | Standard CPU VITS and a matching HTP generator | Call `enable_qnn_pipeline` and then `synthesize_qnn_pipeline` |

You can switch voices on the same model instance. For a reference cache that
spans multiple instances, open a synthesis session with
`TTSModel::from_bundle_lazy_reference`. Call `restore_reference` on a cache hit.
Otherwise, prepare the reference audio and save its features with
`encode_reference`. Lazy mode releases the reference sessions after preparation.
Match cache entries by model bundle, reference WAV, transcript, and `LangId`.

Use `synthesize_token_chunks_with_checkpoints` to share compute resources with
other models. Text features, T2S decoding, and VITS acquire compute permits
through `enter_compute`. The engine retains decoding and overlap state when
compute is paused and releases permits before delivering PCM or waiting for
the next stage. The PCM callback or a cancellation check can end synthesis.

[Rust example](examples/gpt_sovits_demo.rs) ·
[Whole-utterance JNI example](examples/gpt_sovits_demo_jni.rs) ·
[Model validation (Chinese)](scripts/README.md#流式-vits-导出) ·
[Qualcomm QNN (Chinese)](scripts/README.md#qualcomm-qnn-pipeline)

## License

See [LICENSE](LICENSE). The [model card](https://huggingface.co/mikv39/gpt-sovits-onnx-custom#sources-and-licenses)
lists the sources of the checkpoints and text frontend models.
