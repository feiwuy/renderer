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
