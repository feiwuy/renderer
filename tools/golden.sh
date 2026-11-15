#!/bin/sh
# 渲染基准图回归检查
#
# 目的：改动渲染相关代码后，确认画面**逐位没变**。
# 背景：COLONQ_IDLE=0 + 固定参数下渲染是确定性的（同一台机器反复跑 sha256 一致），
#       所以可以把某一帧的哈希当基准。之前踩过两次「画面变了」的坑（黑屏、字符错位），
#       这类问题单元测试抓不到，这个脚本能抓到。
#
# 用法：
#   sh tools/golden.sh            # 对比，不一致则失败
#   sh tools/golden.sh --update   # 用当前渲染刷新基准（确认这是你想要的画面后再跑）
#
# 说明：
#   - 基准**与机器/驱动相关**（显卡、驱动版本不同，浮点结果可能有极微小差异），
#     所以基准文件是「本机基线」，换机器后需要 --update 重建。
#   - 需要 assets/my_avatar.vrm（或用 GOLDEN_MODEL 指定别的模型）；缺模型或没有
#     GL 环境（例如无头 CI）时输出 SKIP 并以 0 退出，避免误报。
set -u
cd "$(dirname "$0")/.." || exit 1

BIN=./target/release/colonq
GOLDEN_DIR=tests/golden
GOLDEN="$GOLDEN_DIR/frame64.sha256"
MODEL="${GOLDEN_MODEL:-my_avatar.vrm}"
GRID="${GOLDEN_GRID:-128x64}"

if [ ! -x "$BIN" ]; then
    echo "SKIP: 找不到 $BIN，先 cargo build --release" >&2
    exit 0
fi
if [ ! -f "assets/$MODEL" ]; then
    echo "SKIP: 缺少 assets/$MODEL（模型不进仓库；用 GOLDEN_MODEL 指定其他模型）" >&2
    exit 0
fi

render() {
    rm -f frame64.png frame-view.png
    env COLONQ_IDLE=0 COLONQ_NO_TERM=1 COLONQ_PREVIEW=0 COLONQ_FPS=0 \
        COLONQ_DUMP_PNG=1 COLONQ_DUMP_AT=120 \
        COLONQ_GRID="$GRID" COLONQ_MODEL="$MODEL" \
        timeout 30 "$BIN" >/dev/null 2>&1 || true
}

render
if [ ! -f frame64.png ]; then
    echo "SKIP: 渲染没有产出 frame64.png（可能没有可用 GL 环境）" >&2
    exit 0
fi

actual=$(sha256sum frame64.png | cut -d' ' -f1)
rm -f frame64.png frame-view.png

if [ "${1:-}" = "--update" ]; then
    mkdir -p "$GOLDEN_DIR"
    {
        echo "# 渲染基准：COLONQ_IDLE=0 / FPS 不限 / DUMP_AT=120 / GRID=$GRID / MODEL=$MODEL"
        echo "# 更新方式：sh tools/golden.sh --update（换机器或换驱动后需要重建）"
        echo "$actual"
    } > "$GOLDEN"
    echo "已更新基准: $GOLDEN → $actual"
    exit 0
fi

if [ ! -f "$GOLDEN" ]; then
    echo "还没有基准文件。确认当前画面正确后运行: sh tools/golden.sh --update" >&2
    exit 1
fi

expected=$(grep -v '^#' "$GOLDEN" | head -1 | tr -d '[:space:]')
if [ "$actual" = "$expected" ]; then
    echo "PASS: 渲染与基准一致（$actual）"
    exit 0
fi

echo "FAIL: 渲染结果与基准不一致" >&2
echo "  基准: $expected" >&2
echo "  实际: $actual" >&2
echo "  如果这是有意改动（例如调整了着色/取景），确认画面正确后运行:" >&2
echo "      sh tools/golden.sh --update" >&2
exit 1
