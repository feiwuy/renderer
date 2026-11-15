#!/bin/sh
# 本地运行脚本（Arch Linux）
# 必须在带图形显示的真终端里执行：程序会建一个 SDL 窗口做 GL 渲染，
# 同时把画面转成字符打到 stdout。
set -e
cd "$(dirname "$0")"

if [ ! -f assets/seed_san.vrm ]; then
    echo "缺少 assets/seed_san.vrm（VRM 1.0 模型）" >&2
    exit 1
fi

# 复用工作区里的 cargo 缓存（本仓库在同级 .cargo-home 里下载过依赖）
if [ -z "$CARGO_HOME" ] && [ -d ../.cargo-home ]; then
    export CARGO_HOME="$(cd .. && pwd)/.cargo-home"
fi

cargo build --release "$@"

# 程序自己会 panic 退出并返回非 0；错误详情写在 colonq.log 里，这里回显出来免得看不到
set +e
./target/release/colonq "$@"
code=$?
set -e
if [ "$code" -ne 0 ]; then
    echo
    echo "!!! 进程退出码 $code，colonq.log 里的错误信息：" >&2
    grep -a -E "ERROR|panicked|libmpv|mpv GL|GL 上下文|shader" colonq.log 2>/dev/null | tail -12 >&2
    echo "（完整日志: $PWD/colonq.log）" >&2
fi
exit "$code"
