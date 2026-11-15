# Ubuntu Server 面捕部署（facetracker 直读手机视频流）

> 面向：Ubuntu Server 实体机，局域网内。**不需要 v4l2loopback、不需要桌面、不需要 sudo 安装内核模块。**
> 原理：DroidCam 手机 App 自带 MJPEG 视频流，OpenSeeFace 用 OpenCV 直接读 HTTP 视频地址（OpenCVReader 支持），
> 省掉虚拟摄像头环节。识别结果以 UDP 发给 Arch 上的 renderer。

## 0. 版本前提

项目自带的 OpenSeeFace（vendored，见 `renderer/deps/OpenSeeFace/pyproject.toml`）要求：

- Python `>=3.7,<3.11`
- numpy `^1.21.3`、opencv-python `^4.5.4`、Pillow `^8.4.0`、onnxruntime `^1.9.0`

下面按这个约束来。**如果服务器是 Ubuntu 24.04+（系统 Python 3.12/3.13），必须走第 2 步装独立 Python**，不要用系统 Python 硬装。

## 1. 手机端：DroidCam App

1. 手机装 DroidCam（Play 商店 / 官网 APK），启动后记下屏幕显示的 **Wi-Fi IP 和端口（默认 4747）**。
2. 手机和 Ubuntu Server 必须在同一局域网。

先自检视频流可用（在服务器上，`<phone-ip>` 换成手机 IP）：

```bash
curl -s -o /dev/null -w '%{http_code} %{content_type}\n' 'http://<phone-ip>:4747/video'   # 期望 200, multipart/x-mixed-replace
```

## 2. 服务器端环境

```bash
# 2a) 需要 Python 3.10/3.11 时的做法（Ubuntu 22.04 自带 3.10 可跳过 2a）
sudo add-apt-repository -y ppa:deadsnakes/ppa && sudo apt update
sudo apt install -y python3.10 python3.10-venv
python3.10 -m venv ~/osf-env
# Ubuntu 24.04 直接用系统 Python 也可以，但要接受 OpenSeeFace 老代码可能需要小修补：
# python3 -m venv ~/osf-env
source ~/osf-env/bin/activate

# 2b) 依赖（不用 poetry，直接 pip）
pip install --upgrade pip
pip install 'numpy<2' 'opencv-python>=4.5.4' 'Pillow>=8.4' 'onnxruntime>=1.9'

# 2c) 从 renderer 仓库拷贝 OpenSeeFace（含 models/，模型文件已在仓库内）
cp -r /path/to/renderer/deps/OpenSeeFace ~/OpenSeeFace
```

自检推理环境：

```bash
cd ~/OpenSeeFace
python - <<'EOF'
import cv2, numpy, onnxruntime
print('cv2', cv2.__version__, '| numpy', numpy.__version__, '| ort', onnxruntime.__version__)
print('providers:', onnxruntime.capi._pybind_state.get_available_providers())
EOF
```

`providers` 出现 `CUDAExecutionProvider` 只是说明有 CUDA 库可用，CPU 上照样能跑（单脸 model 3 约 44fps，够 24fps 需求）。

## 3. 试跑面捕（先接本机，再发往 Arch）

```bash
# 先发到服务器自己，验证视频流和识别都正常
python facetracker.py \
  -c 'http://<phone-ip>:4747/video' \
  -F 24 -W 640 -H 360 --faces 1 -m 2 \
  -i 127.0.0.1 -p 11573 \
  -v 1 --pnp-points 1        # 有桌面才加 -v；纯服务器用 -v 0 --silent 1
```

- 看到 `Confidence[0]: 0.9x` 且眼睛状态 `O,-` 随你眨眼变化 → 视频流和识别都通了。
- 花屏/打不开 → 换 `-c 'http://<phone-ip>:4747/video'` 与 `/mjpegfeed` 两种路径试；或降为 480p。

确认正常后，把目标改成 **Arch 电脑的局域网 IP**：

```bash
python facetracker.py -c 'http://<phone-ip>:4747/video' \
  -F 24 -W 640 -H 360 --faces 1 -m 2 \
  -i 192.168.1.100 -p 11573 --silent 1
```

## 4. Arch 侧 renderer

新接收器支持局域网来源（`COLONQ_TRACK_BIND`）：

```bash
cd renderer
# 在 Arch 上运行；stdout 不是终端时也可以（会自动只推流）
COLONQ_TRACK_BIND=0.0.0.0:11573 COLONQ_IDLE=0 \
  COLONQ_SERVE=8080 COLONQ_NO_TERM=1 \
  nohup ./target/release/colonq >/dev/null 2>&1 &
```

- 只对 Ubuntu Server 的 IP 开放 UDP 11573（ufw 示例）：
  `sudo ufw allow from <server-ip> to any port 11573 proto udp`
- `COLONQ_IDLE=0`：纯面捕首测先关待机动画；**不开这个开关也行**——新版接收器带 2 秒超时，面捕数据在线时待机动画自动让位，断线自动回退。
- 动作幅度：`COLONQ_TRACK_GAIN`（默认1.5，转头更夸张）与 `COLONQ_EXPR_GAIN`（默认1.4，眨眼/张嘴更明显）；觉得过头就调回 1.0。
- 接收器已修 mouth_open 偏移，并带包长度/数值校验和 3 个单元测试（`src/tracking.rs`）。
- OBS 设置不变：`http://127.0.0.1:8080/?width=385&height=350`。

## 5. 验证清单

1. renderer 日志出现 `面捕 UDP 接收: 0.0.0.0:11573`，没有“绑定失败”。
2. 你转头 → OBS 里头部跟随；**方向不对**时再用轴补偿参数调，不要先改模型。
3. 眨眼 → `blink` 生效；张嘴 → `oh` 生效（新代码已修 mouth_open 偏移，1777 处）。
4. 停掉 facetracker 约 2 秒后，OBS 里回到待机摆动（tracking.rs 的超时回退）。
5. `python3 tools/check_stream.py` 应保持约 24fps、无跳序号。

## 6. 已知限制

- mouth_open 只驱动 `oh`；`aa/ih/ee` 等口型在本公开版 renderer 中未接线（上游就这样）。
- 上游四元数轴补偿按作者模型调的；你的 VRM 0.x 适配模型若转向相反，调 `COLONQ_YAW` 或补偿逻辑。
- 服务器重装/内核升级不影响面捕（没有内核模块依赖）；`~/OpenSeeFace` 和 `~/osf-env` 建议纳入你的备份。

## 7. 降 CPU 占用 / 降风扇噪音

面捕跑在服务器上时风扇吵，通常是「持续多核满载」。先搞清几个容易踩的点：

**① `-m` 不是选模型，是线程数**
`-m/--max-threads`（默认 1）控制 onnxruntime 的 intra-op 线程数（内部 `min(max_threads, 4)`）。
模型是 `--model`（默认 **3**，对应 `lm_model3_opt.onnx` 13.5 MB，属于偏重的档位）。
所以 `-m 2` 是"用 2 个线程跑默认模型 3"，不是"用模型 2"。

**② 多线程不会省电，反而更吵**
核心参与得越多，功耗和风扇转速越高。要安静就**限制**线程：`-m 1`。

**③ 默认开着凝视模型，本渲染器用不到**
`--gaze-tracking` 默认 1，每帧多跑一个 `mnv3_gaze32_split` 推理；renderer 只用四元数 / 眨眼 / 张嘴，
用不上凝视 → 设 `--gaze-tracking 0`，白赚。

**④ 定期人脸复检**
`--scan-every`（默认 3）表示每 3 帧重新检测一次人脸。单人、机位固定时可以放大到 10~30。

**⑤ 分辨率和管道带宽**
640×360×3×24 ≈ 16.6 MB/s 要在管道里过。降到 480×270 后解码、resize、带宽都降约 44%。

**⑥ OpenCV 别铺满多核**
`mjpeg_to_raw.py` 新增 `--threads`（默认 1）：限制 OpenCV 内部线程，避免 resize/cvtColor 瞬间占满全部核心。

> ⚠️ 代价是**解码吞吐**：`--threads 1` 时如果 `源 X fps` 掉到 20 以下（正常情况下源应接近 24-25），
> 说明瓶颈在解码而不是推理。这时把 `--threads` 调回 `0`（OpenCV 按核数）或 `2`，
> 帧率会回到 24；推理侧仍保持 `-m 1`，噪音的大头（推理）依然被压住了。
> 想要 30 fps 更顺滑时同理：先确认 `--threads 0` 下源 fps 能到 30，再改 `--fps 30 -F 30`。

### 推荐的低功耗启动组合

```bash
~/osf-env/bin/python mjpeg_to_raw.py --url http://192.168.1.210:4747/video \
    --width 480 --height 270 --fps 24 --threads 1 \
  | ~/osf-env/bin/python facetracker.py --raw-rgb 1 -W 480 -H 270 -F 24 \
    --faces 1 -m 1 --model -1 --gaze-tracking 0 --scan-every 10 \
    -i 192.168.1.100 -p 11573 --silent 1
```

- `-m 1`：推理单线程（最省电）
- `--model -1`：比默认模型 3 轻；不够轻再试 `--model -2`（更快，精度略降）
- `--gaze-tracking 0`：去掉凝视推理
- `--scan-every 10`：减少人脸复检
- `-W/-H` 必须和 `mjpeg_to_raw.py` 的 `--width/--height` 一致

### 怎么挑模型：让它自己报速度

```bash
cd ~/OpenSeeFace && ~/osf-env/bin/python facetracker.py --benchmark 1
```

会依次跑各模型并打印每帧耗时。挑「能达到 24 fps 的最快模型」即可——超过 24 fps 的部分都是白烧 CPU。
也可以直接看 `--log-data`/`top` 的 %CPU 对比改动前后。

降不动、又想要 30 fps 以上时，才考虑加线程（`-m 4`）；代价是风扇会更响。

### 实测选型：`--benchmark 1` 的输出怎么读

`facetracker.py --benchmark 1` 按 `[3, 2, 1, 0, -1, -2, -3]` 的顺序打印 7 个数字，
单位是 **fps（越大越快）**，测试对象是 224×224 的人脸裁切、单线程、**不含人脸检测**；
注意除 `-1` 外都带凝视推理（源码里 `no_gaze=(model_type == -1)`），所以开了
`--gaze-tracking 0` 之后实际都会更快一些。

一台机器上的真实输出示例：

```
55.7  65.2  84.9  101.1  271.9  118.0  153.2
 3     2     1      0      -1     -2     -3
```

| `--model` | 实测 fps | 每帧耗时 | 说明 |
|---|---|---|---|
| `3`（默认） | 55.7 | 18.0 ms | 质量最高 |
| `2` | 65.2 | 15.3 ms | 略快，质量接近 3 |
| `1` | 84.9 | 11.8 ms | 上游说 1/0 对**表情和眨眼偏僵硬** |
| `0` | 101.1 | 9.9 ms | 最快但质量最低 |
| `-2` | 118.0 | 8.5 ms | 大致等价 model 1 但更快 |
| `-3` | 153.2 | 6.5 ms | 介于 0 和 -1 之间 |
| `-1` | 271.9 | 3.7 ms | 上游原话：给"烤面包机"用的极速低精度模型 |

**怎么选**：只要 24 fps（每帧预算 41.7 ms），模型 3 就占了 18 ms。

1. 先用 `--model 2`：质量几乎不变，省约 15%
2. 还吵就 `--model -2`：模型开销再砍一半（8.5 ms），眨眼/张嘴一般还能接受
3. `-3` / `-1` 只在"确认眨眼和张嘴还能看"的前提下使用——`-1` 是低精度模型，
   表情会明显变钝

每换一档都要回看两件事：端到端还是 24 fps、眨眼和张嘴是否还跟得上。
