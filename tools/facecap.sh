#!/bin/bash
# 面捕启动脚本（Ubuntu Server）—— 低功耗 / 低噪音版本
#
# 用法：./facecap.sh
# 想临时改参数就用环境变量覆盖，例如：
#   PHONE_IP=192.168.1.211 ./facecap.sh
#   MODEL=-2 ./facecap.sh
#
# 关键点（详见 renderer/UBUNTU_FACECAP.md 第 7 节）：
#   -m 1                  ← 是「推理线程数」，不是模型！单线程 = 最省电、最安静
#   --model 2             ← 真正的模型选择（3 最准、0 最快；-2 更快且质量尚可）
#   --gaze-tracking 0     ← renderer 用不到凝视数据，关掉白赚 CPU
#   --scan-every 10       ← 单人固定机位不必每 3 帧重新检测人脸
#   --threads 0           ← 解码交给 OpenCV 自己安排线程（设 1 会拖低帧率）
#
# 输出同时写进 ~/facecap.log（可用 FACECAP_LOG 改路径），超过 5MB 自动滚成 .1
set -u

# ---------------- 可调参数 ----------------
PHONE_IP="${PHONE_IP:-192.168.1.210}"   # 手机 DroidCam 的 IP
ARCH_IP="${ARCH_IP:-192.168.1.100}"     # 跑 renderer 的 Arch 机器 IP
W="${W:-480}"                           # 采集/推理分辨率（mjpeg 与 facetracker 必须一致）
H="${H:-270}"
FPS="${FPS:-24}"                        # 目标输出帧率
MODEL="${MODEL:-2}"                     # --model：3 最准，2 接近 3，-2 更快，-1 极速低精度
THREADS_INFER="${THREADS_INFER:-1}"     # facetracker -m（推理线程数）
THREADS_DECODE="${THREADS_DECODE:-0}"   # mjpeg_to_raw --threads（0 = OpenCV 默认）
SCAN_EVERY="${SCAN_EVERY:-10}"
GAZE="${GAZE:-0}"                       # --gaze-tracking
# ------------------------------------------

OSF="$HOME/OpenSeeFace"
PY="$HOME/osf-env/bin/python"

# 只杀「这个 venv 的 python 跑这两个脚本」的进程。
# 别写成 pkill -f mjpeg_to_raw —— 那样会连带杀掉任何命令行里出现过该文件名的进程
# （编辑器、grep、甚至是调用本脚本的 shell）。
pkill -f "$PY mjpeg_to_raw.py" 2>/dev/null
pkill -f "$PY facetracker.py" 2>/dev/null
sleep 1

cd "$OSF" || { echo "找不到 $OSF" >&2; exit 1; }

# 日志：同时打到终端和文件。facetracker 每 10 秒会打印一行源/输出帧率统计，
# 风扇和延迟的问题基本都靠这几行判断。超过 5 MB 就滚动一份 .1（只留最近两份）。
LOG="${FACECAP_LOG:-$HOME/facecap.log}"
[ -f "$LOG" ] && [ "$(stat -c %s "$LOG" 2>/dev/null || echo 0)" -gt 5242880 ] && mv -f "$LOG" "$LOG.1"

# 注意：这里是**一条**管道，两个 python 之间必须有 `|`。
# 之前手动编辑时漏掉 `|` 会导致 facetracker 的参数被当成 mjpeg_to_raw 的参数。
{
    echo "=== $(date '+%F %T') facecap 启动：${W}x${H}@${FPS} model=$MODEL 推理线程=$THREADS_INFER 解码线程=$THREADS_DECODE ==="
    "$PY" mjpeg_to_raw.py \
            --url "http://$PHONE_IP:4747/video" \
            --width "$W" --height "$H" --fps "$FPS" --threads "$THREADS_DECODE" \
      | "$PY" facetracker.py \
            --raw-rgb 1 -W "$W" -H "$H" -F "$FPS" \
            --faces 1 -m "$THREADS_INFER" --model "$MODEL" \
            --gaze-tracking "$GAZE" --scan-every "$SCAN_EVERY" \
            -i "$ARCH_IP" -p 11573 --silent 1
} 2>&1 | tee -a "$LOG"
