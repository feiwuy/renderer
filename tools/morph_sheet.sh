#!/bin/sh
# 把脸 mesh 的 morph target 逐个渲染出来，用于"这 57 个 target 到底是什么表情"。
#
# 用法：
#   sh tools/morph_sheet.sh                  # 渲染全部 target（0..56，比较慢）
#   sh tools/morph_sheet.sh 23 33 44 45      # 只渲染这几个
#   sh tools/morph_sheet.sh 0 56             # id 也可以用区间 a b（含两端）
#   LEVEL=2 sh tools/morph_sheet.sh 23       # 放大倍数（默认 2，看形变更清楚）
#   VIEW=head sh tools/morph_sheet.sh        # 取景（默认 face）
#
# 原理：项目只有 11 个顶点属性槽（已用满），所以"显示第 N 个 target"必须借道一个
# 用不上的槽位——这里统一借用 `happy`：
#   COLONQ_EXPR_MAP="happy=N"    把 happy 槽改绑到第 N 个 morph target
#   COLONQ_EXPR_FORCE="happy=2"  把它的权重钉死在 2（不夹取，方便看放大后的形变）
#
# 产物：tests/morph-sheet/<序号>.png（原始网格）+ <-view.png>（字符比例视图）
# 注意：morph target 的**下标**由导出顺序决定，各模型不一样；本仓库的
#       assets/my_avatar.vrm 的对应关系见 FACE_CAPTURE.md 第 10 节。
set -u
cd "$(dirname "$0")/.." || exit 1

BIN=./target/release/colonq
OUT=tests/morph-sheet
LEVEL="${LEVEL:-2}"
VIEW="${VIEW:-face}"
GRID="${GRID:-128x64}"

if [ ! -x "$BIN" ]; then
    echo "先 cargo build --release" >&2
    exit 1
fi
mkdir -p "$OUT"

render_one() {
    idx=$1
    rm -f frame64.png frame-view.png
    env COLONQ_EXPR_MAP="happy=$idx" \
        COLONQ_EXPR_FORCE="happy=$LEVEL" \
        COLONQ_VIEW="$VIEW" COLONQ_IDLE=0 COLONQ_NO_TERM=1 COLONQ_PREVIEW=0 \
        COLONQ_NO_MPV=1 COLONQ_FPS=24 COLONQ_GRID="$GRID" \
        COLONQ_DUMP_PNG=1 COLONQ_DUMP_AT=24 COLONQ_DUMP_EXIT=1 \
        timeout 60 "$BIN" >/dev/null 2>&1
    if [ -f frame64.png ]; then
        mv frame64.png "$OUT/$idx.png"
        [ -f frame-view.png ] && mv frame-view.png "$OUT/$idx-view.png"
        echo "  target $idx → $OUT/$idx-view.png"
    else
        echo "  target $idx 渲染失败（看 colonq.log）" >&2
    fi
}

if [ $# -eq 0 ]; then
    set -- $(seq 0 56)
elif [ $# -eq 2 ]; then
    set -- $(seq "$1" "$2")
fi

echo "渲染 morph target（放大 $LEVEL 倍，取景 $VIEW）→ $OUT/"
for i in "$@"; do
    render_one "$i"
done
echo "完成。看 $OUT/*-view.png 即可，注意 8、11 这类空 target 不会有任何变化。"
