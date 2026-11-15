#!/bin/bash
# 检测 GL 实际用的是哪个渲染器（直接读 GL_RENDERER，最可靠）
# 用法: ./tools/check-gpu.sh
#
# 会依次尝试几种"强制走独显"的组合，报告每种组合下 GL 报出来的真实渲染器。
cd "$(dirname "$0")/.." || exit 1
BIN="$PWD/target/release/colonq"
[ -x "$BIN" ] || { echo "先编译: cargo build --release"; exit 1; }

echo "=== 设备 ==="
ls /dev/dri 2>/dev/null | tr '\n' ' '; echo
lspci 2>/dev/null | grep -iE 'vga|3d|display' | sed 's/^/  /'
echo

probe() {
    local label="$1"; shift
    pkill -x colonq 2>/dev/null; sleep 0.4
    rm -f colonq.log
    env "$@" DISPLAY="${DISPLAY:-:0}" COLONQ_NO_MPV=1 COLONQ_NO_TERM=1 \
        setsid script -qec "stty cols 200 rows 80; exec $BIN" /dev/null >/dev/null 2>&1 &
    sleep 3
    local line
    line=$(grep -m1 'GL 后端' colonq.log 2>/dev/null | sed -E 's/.*GL 后端: //')
    if [ -n "$line" ]; then
        printf '%-22s %s\n' "$label" "$line"
    else
        local err
        err=$(grep -m1 -E 'ERROR|panic' colonq.log 2>/dev/null | cut -c1-100)
        printf '%-22s 启动失败 %s\n' "$label" "${err:+（$err）}"
    fi
    pkill -x colonq 2>/dev/null; sleep 0.4
}

echo "=== 各种后端组合实际使用的 GL 渲染器 ==="
probe "默认"
probe "PRIME offload"  __NV_PRIME_RENDER_OFFLOAD=1 __GLX_VENDOR_LIBRARY_NAME=nvidia
probe "禁用软件渲染"   LIBGL_ALWAYS_SOFTWARE=0
probe "强制 radeonsi"  MESA_LOADER_DRIVER_OVERRIDE=radeonsi
probe "SDL 走 X11"     SDL_VIDEODRIVER=x11
echo
echo "判读："
echo "  * 出现 llvmpipe / softpipe / swrast  → 软件渲染（没走显卡）"
echo "  * 出现 NVIDIA / GeForce / RTX       → 独显"
echo "  * 出现 AMD / Radeon / Phoenix/rembrandt → 核显"
echo
echo "选定组合后，测占用："
echo "  ./tools/measure.sh 128x64 6"
