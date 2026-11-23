# 模型导出与部署

从 GPT-SoVITS 检查点导出模型时，先生成普通 ONNX 包，再按需要构建流式 VITS。
已有模型包可以直接使用[项目快速开始](../Readme.md#快速开始)或[校验命令](#流式-vits-导出)。

## 从权重导出

在 GPT-SoVITS ONNX RS 项目根目录运行。macOS 开发机需安装 Git、Rust、CMake、FFmpeg 和 `uv`。
设备运行所需的 Android 工具链见 [Android 真机指南](../Readme.md#android-真机构建与使用)。

```bash
export ENV_MANAGER=uv
./scripts/export/01_clone.sh --upstream-ref 04a44eb977586645f33594b72af13a035a2056c6
./scripts/export/02_patch.sh
./scripts/export/03_setup_env.sh --source HF
./scripts/export/04_download_models.sh --version v2Pro
```

已有 `gpt-sovits-upstream/.venv` 且具备上游依赖和 [ONNX 工具](requirements.txt)时，
可复用该环境执行后续步骤。下面的命令生成 `mobile` 普通包。
使用 `quality` 作为精度优先配置时，将 `--profile mobile` 换成 `--profile quality`。

```bash
./scripts/export/05_export.sh --version v2Pro --export-name v2pro-mobile \
  --quant --profile mobile --shared-kv --preoptimize-cpu
MODEL_DIR=gpt-sovits-upstream/onnx-patched/v2pro-mobile
PYTHON=gpt-sovits-upstream/.venv/bin/python
```

### 生成流式模型包

保留上面的 `$MODEL_DIR` 作为整段推理用包。先拆出参考音图，再生成流式 VITS，
两步分别输出到新的目录。

```bash
CACHED_DIR="${MODEL_DIR}-cached"
STREAM_DIR="${MODEL_DIR}-stream"
"$PYTHON" scripts/vits_dynamic.py "$MODEL_DIR/vits.onnx" --split-reference-dir "$CACHED_DIR"
rsync -a --exclude='/vits.onnx' --exclude='/vits_ref.onnx' \
  --exclude='/ref.wav' --exclude='/ref.txt' --exclude='/reference.wav' --exclude='/reference.txt' \
  "$MODEL_DIR/" "$CACHED_DIR/"
mkdir -p "$STREAM_DIR"
rsync -a --exclude='/vits.onnx' "$CACHED_DIR/" "$STREAM_DIR/"
"$PYTHON" scripts/vits_streaming.py "$CACHED_DIR/vits.onnx" "$STREAM_DIR/vits.onnx"
./scripts/export/validate_bundle.sh --python "$PYTHON" \
  --bundle-dir "$STREAM_DIR" --expect-v2pro --expect-streaming
```

用流式推理 CLI 时，将 `--model-path` 指向 `$STREAM_DIR` 并传入 `--stream`。
整段推理可使用 `$MODEL_DIR` 或拆出参考图后的 `$CACHED_DIR`。

## 流式 VITS 导出

普通包和流式包都以 `vits.onnx` 为主图文件名，图的输入和调用入口如下。

| 模型包 | 生成步骤 | VITS 输入 | 使用方式 |
| --- | --- | --- | --- |
| 普通包 | `05_export.sh` | 文本、语义 token、参考音和可选说话人特征，拆分参考音后使用 `ge` | 非流式。CLI 默认模式或 Rust `synthesize_sync` 返回完整 PCM |
| 流式包 | 参考音拆分后运行 `vits_streaming.py` | 在普通图基础上增加 `stream_start_frame` | CLI `--stream` 或 Rust `synthesize_token_chunks_sync`，按 Adaptive 逐块返回 PCM |

校验脚本用 `--expect-ordinary` 或 `--expect-streaming` 检查图类型，`--expect-v2pro` 检查
Pro 系列模型接口和说话人模型。已有模型包可以用本地已安装的 `onnx` 和 `onnxruntime` 校验，
通过 `--python /path/to/python` 指定对应解释器。缺少流式图时，流式接口返回错误。

```bash
./scripts/export/validate_bundle.sh --bundle-dir "$MODEL_DIR" --expect-v2pro --expect-ordinary
./scripts/export/validate_bundle.sh --bundle-dir "$STREAM_DIR" --expect-v2pro --expect-streaming
```

流式图保留语义前缀，按起始帧裁切 flow 和生成器的计算范围。Adaptive 根据播放余量和近期
合成耗时调整后续块长。默认前瞻 6 个 token，拼接重叠 2 个 token。命令行的
`--block-tokens` 默认 8，控制首块和后续块长下限。使用 `vits_streaming.py --latent-history`
可导出实验性的 flow 历史复用图，由 Rust 调用方通过 `TokenStreamOptions` 设置左侧历史。

## 参考音特征缓存与 CPU 预融合

v2Pro 和 v2ProPlus 可以用 `vits_dynamic.py --split-reference-dir` 将 VITS 拆成
`vits_ref.onnx` 和使用 `ge` 的 `vits.onnx`。引擎在加载或切换参考音时计算一次
`ge`，后续合成复用。将其他 ONNX 文件复制到拆图输出目录后，即可用 `$CACHED_DIR` 进行整段推理。

`--preoptimize-cpu` 在量化之后对 BERT、中英文 G2P 和适用的 T2S 图做 ORT Extended
预融合。已有模型包可用 `python scripts/preoptimize_cpu.py "$CACHED_DIR"` 更新。
`--shared-kv` 为 T2S 逐 token 解码启用可复用的 KV 缓冲区。运行时通过
`past_length` 输入识别共享布局，并按长句需要扩容。[CPU 优化实测](../doc/perf_record.md#cpu-优化)

## Android CPU 低延迟导出

`mobile` 是 Android CPU 部署配置。`05_export.sh` 默认输出浮点图，传入 `--quant`
后应用所选量化配置。

| 配置 | BERT / G2PW | T2S 首步 | T2S 逐 token | VITS |
| --- | --- | --- | --- | --- |
| `quality` | 分块 INT8 权重，FP32 激活 | FP32 | FP32 | FP32 |
| `mobile` | 动态 INT8 投影 | 动态 INT8 投影 | 分块 INT4 投影，FP32 词表输出层 | 默认 FP32，可选卷积 INT8 校准 |
| `rtf` | 同 `quality` | FP32 | 分块 INT4 投影，FP32 词表输出层 | FP32 |
| `compact` | 分块 INT4 权重 | 动态 INT8 | 动态 INT8 | FP32 |

已有代表性输入时，给 `--quant --profile mobile` 加上
`--vits-calibration-dir /path/to/calibration`，校准 VITS 前九个残差块的卷积。
每个 `.npz` 包含 `text_seq`、`pred_semantic`、`ref_audio` 和 Pro 系列的 `sv_emb`。
更低位宽的 INT2、INT3 属于实验候选。评估时可对照图大小、首包和连续合成耗时、
语义 token、PCM、字错率及音色相似度。

## Android App 接入

交叉编译目标为 `aarch64-linux-android`（`arm64-v8a`、API 28）。`build_for_android.sh`
输出的 `libonnxruntime.so` 来自全量 ORT 1.30.0 Android AAR，支持量化图用到的 contrib
算子。将 ORT、`libc++_shared.so` 和应用自己的 JNI 库放进 Gradle 的
`src/main/jniLibs/arm64-v8a/` 或通过 `sourceSets` 指定目录。使用绝对文件路径
`ort::init_from` 时，可采用 `jniLibs.useLegacyPackaging = true`，安装后从
`applicationInfo.nativeLibraryDir` 取得 ORT 文件。交付 APK 时检查各 AArch64 ELF 的
16 KiB 页对齐与 APK 的原生库条目。

### 整段 JNI 示例

[`gpt_sovits_demo_jni.rs`](../examples/gpt_sovits_demo_jni.rs) 演示从 Java 传入模型路径、
参考音和文本，再取得完整的 32 kHz 单声道 Float32 PCM。它导出的 JNI 名称对应
`com.example.gpt_sovits_demo.MainActivity`，移植到自己的包名时同步调整符号。
配套的 Java 声明如下，方法放在该类中。

```java
static { System.loadLibrary("gpt_sovits_demo_jni"); }

static native long initModel(String g2pW, String g2pEnDir, String vits, String ssl,
    String t2sEncoder, String t2sFirstDecoder, String t2sStepDecoder, String bert, String sv);
static native boolean processReferenceSync(long model, String wav, String transcript, long language);
static native float[] runInferenceSync(long model, String text, long language);
static native void freeModel(long model);
```

用普通 VITS 图初始化。v2Pro 系列的 `sv` 传入 `sv.onnx` 的路径，v2 传入空字符串。
语言值 `0` 选择普通话／英文自动处理，`1` 选择粤语模式。宿主在工作线程依序初始化、
处理参考音、合成和释放；模型路径指向磁盘上的文件，`g2pEnDir` 指向 `g2p_en/`。
共享线程池的初始化顺序可以参考 [Android CLI](../examples/gpt_sovits_demo.rs) 的
`init_android_ort`。

### Adaptive 流式宿主

使用带 `stream_start_frame` 的流式包并保留模型目录结构，包括 `g2p_en/`、
拆分参考图 `vits_ref.onnx` 和模型引用的外部数据文件。可先运行
`validate_bundle.sh --bundle-dir PATH --expect-v2pro --expect-streaming` 核对模型。
宿主在创建任何会话前从本地库路径初始化 ORT，选择 `shared-ort-pool` 时同时配置全局线程池。
用 `TTSModel::from_bundle_lazy_reference` 和 `process_reference_sync` 准备模型及参考音，
再调用 `synthesize_token_chunks_sync`。用 `InferParams::from_bundle` 读取包内的采样参数，
以回调中的 `WavSpec` 配置播放端。

Android `TextToSpeechService` 可把 Float32 PCM 转成小端 PCM16，按
`SynthesisCallback.getMaxBufferSize()` 分块交给系统。模型的 PCM 回调可能在 Rust 工作线程执行，
通过有界队列把 PCM 送回 Android 合成线程后再调用系统回调；`onStop` 设置合成的取消状态。
[Android 回调契约](https://developer.android.com/reference/android/speech/tts/SynthesisCallback)

## 运行参数与验证

- **采样。** 默认 `top_k=4`、`top_p=0.9`、`temperature=1.0`、`repetition_penalty=1.35`。
  命令行可覆盖这些值及 `--seed`，也可用 `--params` 指定 JSON。默认读取模型目录的
  `infer_params.json`，种子未设时随机采样。`--lang yue` 选择粤语模式。
- **参考音。** WAV 和转录由运行时提供。用 `--ref-audio` 指定外部 WAV，或使用当前工作目录的
  `ref.wav`。Pro 系列模型包包含 `sv.onnx`。导出自测通过
  `05_export.sh --smoke-test --ref-audio … --ref-text … --text …` 直接读取外部参考音。
- **线程。** 默认每个 ORT 会话使用独立线程池。宿主可启用 `shared-ort-pool`，并在加载模型前
  用 `GlobalThreadPoolOptions` 配置共享池。Android CLI 使用 4 个 ORT 池线程。
  `configure_onnx_sessions` 可在加载模型前设置等待自旋。
- **回归。** `cargo test --release --lib` 验证 Rust 逻辑。
  `python -m unittest discover -s scripts -p test_vits_dynamic.py` 验证动态 VITS 图。
  `compare_onnx_versions.py` 和 `compare_infer_metrics.py` 用同一个 `--ref-audio` 对照两端，
  输出位于 `scripts/reports/`。
  [性能记录](../doc/perf_record.md) · [v2ProPlus 验证](../doc/v2proplus_validation.md)

## 其他模型与自定义导出

分阶段脚本在 macOS 上运行。步骤 02 将子仓库的 `patch/GPT_SoVITS/` 覆盖到上游，以导出 Rust 所需的外部 KV。
推荐用 `uv` 准备导出环境。步骤 03 在 `gpt-sovits-upstream/.venv` 中安装上游 Python 依赖和
[ONNX 工具](requirements.txt)。已有该环境且依赖齐全时，可以跳过步骤 03。已有普通 ONNX
模型包时，拆图和校验也可以直接使用装有 `onnx` 和 `onnxruntime` 的本地 Python。
步骤 04 支持 `v2`、`v2Pro`、`v2ProPlus`。官方 v2Pro 权重为 `s1v3.ckpt` 和
`v2Pro/s2Gv2Pro.pth`。自定义检查点可通过 `05_export.sh --model-path PATH` 指定目录。

`--vits-only` 可用同权重刷新已有普通包的 VITS。旧 VITS 图可用
`vits_dynamic.py INPUT --output OUTPUT` 修复固定长度掩码。自定义或手动导出的包可用
`validate_bundle.sh` 核对 VITS、T2S、参考音和中英文 G2P 等必需文件。

## Qualcomm QNN pipeline

QNN 是 Android 上的可选后端。它使用普通 CPU VITS 包及配套的 128 帧 W8A16 HTP 生成器。
CPU 生成整句语义 token 和 VITS 潜变量，HTP 每完成一个窗口便交付裁边后的 PCM。
Bailu 的对话路径使用 CPU Adaptive。独立 CLI 通过 `--model-path` 和 `--qnn-libraries` 显式启用
QNN，Rust 调用方使用 `enable_qnn_pipeline` 后调用 `synthesize_qnn_pipeline`。

### 下载运行库并构建

```bash
rustup target add aarch64-linux-android
export ANDROID_NDK_ROOT=/path/to/android-ndk
./build_for_android.sh --qnn
```

脚本从 Maven Central 获取并校验 ORT、QNN provider 和 QAIRT 归档。使用
`--output-dir` 指定产物目录，`--archive-dir` 复用下载缓存。只准备运行库时可以运行
`./scripts/prepare_android_runtime.sh --output-dir target/android/lib --qnn`。
S25 Ultra 实测环境为 ORT 1.30.0、QNN EP 2.7.0、QAIRT 2.51.0 和 burst 模式。

### 准备同权重模型和校准输入

使用上面的 `$CACHED_DIR` 作为普通 CPU 包。用同一套权重另导出完整浮点 VITS，
保留 `ConvTranspose`，作为 HTP 生成器的输入。准备多条代表性中长句，每个参与校准的模型段
至少需要 64 个语义 token。

```bash
./scripts/export/05_export.sh --version v2Pro --export-name v2pro-qnn-fp32 \
  --no-quant --vits-conv-transpose
QNN_SOURCE=gpt-sovits-upstream/onnx-patched/v2pro-qnn-fp32/vits.onnx

mkdir -p target
cargo run --release --example gpt_sovits_demo -- \
  --model-path "$CACHED_DIR" --ref-audio /path/to/ref.wav --ref-text '<参考音频的准确转录>' \
  --text '<多条代表性中长句>' --seed 42 --dump-qnn-inputs target/qnn-calibration.json
"$PYTHON" scripts/vits_generator.py "$QNN_SOURCE" target/qnn-generator \
  --frames 128 --htp-calibration-inputs target/qnn-calibration.json --htp-activation-bits 16

QNN_DIR=target/qnn-bundle
mkdir -p "$QNN_DIR"
rsync -a "$CACHED_DIR/" "$QNN_DIR/"
cp target/qnn-generator/vits_prefix.onnx target/qnn-generator/vits_generator_htp.onnx "$QNN_DIR/"
./scripts/export/validate_bundle.sh --python "$PYTHON" --bundle-dir "$QNN_DIR" --expect-qnn
```

可重复传入 `--htp-calibration-inputs` 合并多份 JSON，或用 `--htp-calibration-dir` 指定
现成的 `z`、`ge` 校准窗口。已验证的模型转换环境为 ONNX 1.23.2 和 ONNX Runtime 1.30.0。
`--expect-qnn` 校验普通 CPU VITS、HTP 边界及已有的 `EPContext`。

### 真机运行并生成 EPContext

```bash
DEVICE_DIR=/data/local/tmp/gpt-sovits-qnn
adb shell "mkdir -p '$DEVICE_DIR'"
adb push target/android/gpt_sovits_demo "$DEVICE_DIR/gpt_sovits_demo"
adb push target/android/lib "$DEVICE_DIR/lib"
adb push "$QNN_DIR" "$DEVICE_DIR/models"
adb push /path/to/ref.wav "$DEVICE_DIR/ref.wav"
adb shell "chmod 700 '$DEVICE_DIR/gpt_sovits_demo'"
adb shell "LD_LIBRARY_PATH='$DEVICE_DIR/lib' ADSP_LIBRARY_PATH='$DEVICE_DIR/lib' \
  '$DEVICE_DIR/gpt_sovits_demo' --model-path '$DEVICE_DIR/models' --qnn-libraries '$DEVICE_DIR/lib' \
  --ref-audio '$DEVICE_DIR/ref.wav' \
  --ref-text '<参考音频的准确转录>' --text '<至少一条中长句>' \
  --seed 42 --run-count 4 --output '$DEVICE_DIR/qnn.wav'"
adb pull "$DEVICE_DIR/qnn.wav" target/qnn.wav
```

首次运行可加 `--compile-qnn-context` 生成设备缓存。后续运行会优先加载该缓存。
`GptSovitsQnn` 日志的 `htp_ready` 和 `htp_sentence` 记录 HTP 启用情况和窗口计时。
复现性能表时丢弃预热轮，取后三轮中位数。PCM 对照需固定 `z`、`ge`，T2S 的 `--seed`
只固定语义采样。[QNN 实测与测试语料](../doc/perf_record.md#qualcomm-qnn-pipeline)
