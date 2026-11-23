# GPT-SoVITS ONNX RS

GPT-SoVITS ONNX RS 是 [GPT-SoVITS](https://github.com/RVC-Boss/GPT-SoVITS) 的优化推理库。
项目提供改进的 ONNX 导出脚本和基于 Rust、ONNX Runtime 的 CPU 推理运行时，支持 x86-64
和 arm64。以 v2Pro 为例，实测 CPU 句内流式合成在 Galaxy S25 Ultra 上约为实时的 **3.7 倍**，
在 Mac M4 Pro 上约为实时的 **9.3 倍**。

给定参考 WAV、它的准确转录和待朗读文本，库以参考音色生成 32 kHz 单声道音频。Rust 应用
可以逐块接收 PCM 或一次取回整段音频，命令行示例可以用同一套模型直接生成 WAV。

## 项目做了哪些优化

- **重新组织 ONNX 导出。** 将 T2S 首步和逐 token 解码拆成独立图，显式导出 KV 状态，让
  逐 token 解码复用缓冲区。导出工具可对 BERT、G2PW 和 T2S 采用不同的量化配置，并将适合
  CPU 的图融合提前到导出阶段。`mobile` 可用 INT4 权重减轻逐 token 投影的开销。
  [导出与量化配置](scripts/README.md#android-cpu-低延迟导出)
- **复用参考音计算。** 将 VITS 的参考音条件提取拆成独立图。同一音色的多次合成复用其
  条件向量和语义提示，宿主也可以跨模型实例保存和恢复这些特征。
- **在一句话内部并行生成 PCM。** 流式 VITS 接收帧偏移，渲染当前波形块及拼接所需的上下文，
  T2S 同时继续解码。Adaptive 根据近期合成时间和预计播放余量选择后续块长。

## CPU 性能

下表使用 v2Pro 流式 ONNX，在相同的 25 条文本上热机运行，首块从 8 个语义 token 起步。

| 设备 | 平均首块 PCM | 平均 RTF |
| --- | ---: | ---: |
| Mac M4 Pro | 175 ms | 0.108 |
| Galaxy S25 Ultra | 386 ms | 0.271 |

RTF 是合成耗时除以生成的音频时长，数值越低越快。两台设备在这组测试里均无模拟播放断供。
S25 Ultra 另一次多句测试以 RTF 0.219 生成了 72 秒音频。上百秒的单句实测出现过重复和漏读，
长文适合按自然句提交。[测试设置、各场景结果及 x86-64 历史数据](doc/perf_record.md)

## 支持范围

| 项目 | 支持情况 |
| --- | --- |
| 模型 | v2、v2Pro、v2ProPlus 整段推理。v2Pro CPU 句内流式已在 macOS 和 Android 实测 |
| 语言 | 普通话、英文及中英混读。粤语可使用 CLI 的 `--lang yue` 或 Rust 的 `LangId::AutoYue` |
| 输出 | 流式接口逐块交付 PCM，整段接口一次交付完整 PCM。CLI 将输出写为 WAV |
| 平台 | x86-64 和 arm64 CPU。Android 示例面向 `arm64-v8a` |
| 自定义音色 | 参考 WAV 与准确转录可在运行时更换，Pro 系列使用 `sv.onnx` 提取说话人特征 |

## 快速开始

公开的 [v2Pro 模型仓库](https://huggingface.co/mikv39/gpt-sovits-onnx-custom)提供两个独立目录。
桌面用 `quality/`，Android 用 `mobile/`。下面在 macOS 上用 `quality` 生成第一段 WAV。
需要稳定版 Rust、CMake、Xcode Command Line Tools 和 Hugging Face CLI。
示例用 `uvx` 启动 CLI，也可以使用已安装的 `hf`。

```bash
MODEL_ROOT="$PWD/models/gpt-sovits"
uvx --from huggingface_hub hf download mikv39/gpt-sovits-onnx-custom \
  --include 'quality/**' --local-dir "$MODEL_ROOT"
MODEL_DIR="$MODEL_ROOT/quality"
```

准备一段建议 3–10 秒的单声道 PCM16 WAV，把 `--ref-text` 换成它的准确转录。

```bash
cargo run --release --example gpt_sovits_demo -- \
  --model-path "$MODEL_DIR" --stream \
  --ref-audio /path/to/voice.wav --ref-text '<参考音转录>' \
  --text '等雨停下来以后，我们沿着河边走到灯塔。' \
  --output speech.wav
```

- **生成整段音频**可以使用本目录的 `vits_non_streaming.onnx`。[整段模型运行](https://huggingface.co/mikv39/gpt-sovits-onnx-custom#run)
- **在 Android 上运行**从下面的[真机构建与使用](#android-真机构建与使用)继续。
- **导出自己的权重或量化模型**从[模型导出](scripts/README.md#从权重导出)开始。

## Android 真机构建与使用

[Android TTS Demo](https://github.com/bailu-obj/gpt-sovits-android-tts) 是基于 Kotlin 和 Compose
Material 3 的示例应用。可以用 v2Pro `mobile/` 模型试听文字、切换参考音、比较流式与整段合成的
推理耗时，也可以设为 Android 的系统 TTS 引擎。Demo 的 README 提供 APK 构建和模型部署步骤。

### 在真机上运行命令行示例

准备稳定版 Rust、CMake、原生构建工具、Android NDK、`curl`、`unzip` 和 `adb`。
开发机可用 macOS 或 x86-64 Linux，目标为 `arm64-v8a`、API 28。下面下载适用于 Android CPU
的 `mobile/`。如需自定义模型，可按[导出说明](scripts/README.md#从权重导出)生成自己的流式包。

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
  --ref-audio '$DEVICE_DIR/ref.wav' --ref-text '<参考音转录>' \
  --stream --text '这段声音由 Android 设备生成。' --output '$DEVICE_DIR/speech.wav'"
adb pull "$DEVICE_DIR/speech.wav" target/android/speech.wav
```

构建脚本提供 CLI、全量 ONNX Runtime 1.30.0、`libc++_shared.so` 和整段 JNI 示例，
真机验证使用 NDK r29（29.0.14206865）。可用 `--output-dir` 指定产物目录，用
`--archive-dir` 复用下载缓存。[Android App 接入](scripts/README.md#android-app-接入)

## Rust 接口

用 `TTSModel::from_bundle` 打开模型包，再用 `process_reference_sync` 处理参考音。
`InferParams::from_bundle` 读取模型附带的采样设置。

| 输出方式 | VITS 图 | 调用 |
| --- | --- | --- |
| 流式 | 带 `stream_start_frame` 的流式 VITS | `synthesize_token_chunks_sync` 在回调中交付 `WavSpec` 和 PCM |
| 整段 | 普通 VITS | `synthesize_sync` 返回 `(WavSpec, Vec<f32>)` |
| Qualcomm HTP | 普通 CPU VITS 和配套 HTP 生成器 | `enable_qnn_pipeline` 后使用 `synthesize_qnn_pipeline` |

反复更换音色时可以保留同一个模型实例。宿主若希望跨实例复用参考特征，可用
`TTSModel::from_bundle_lazy_reference` 打开合成会话。缓存命中时调用 `restore_reference`，
否则处理参考音后用 `encode_reference` 保存特征。按需模式在参考音处理结束后释放参考会话。
宿主按模型包、参考 WAV、转录和 `LangId` 匹配缓存。

需要和其他模型调度同一批计算资源时，用 `synthesize_token_chunks_with_checkpoints`。
文本特征、T2S 解码和 VITS 会通过 `enter_compute` 获取许可。计算暂停时保留解码、
拼接状态，交付 PCM 或等待下一阶段前释放许可。PCM 回调与取消条件可结束当前合成。

[Rust 示例](examples/gpt_sovits_demo.rs) · [整段 JNI 示例](examples/gpt_sovits_demo_jni.rs) ·
[模型校验](scripts/README.md#流式-vits-导出) · [Qualcomm QNN](scripts/README.md#qualcomm-qnn-pipeline)

## 许可证

参见 [LICENSE](LICENSE)。模型检查点及文本前端模型的来源见
[模型说明](https://huggingface.co/mikv39/gpt-sovits-onnx-custom#sources-and-licenses)。
