#!/bin/sh
# 启动脚本体检：语法 + 每一行赋值能不能真正执行
#
# 为什么需要：`sh -n` 只做语法检查。`export A=0.75 / B=0.95` 这种写法**语法是对的**
# （就是 export 后面跟了几个词），要到执行时才报 "not a valid identifier" ——
# 之前就踩过：把注释里的示例取消注释后，脚本跑到那一行直接报错、渲染器起不来。
#
# 做法：
#   1. 所有 .sh 过一遍 `sh -n`（语法）
#   2. run-obs.sh 里每一条未注释的 `export …` 单独丢给 sh 执行一遍（语义）
#      —— 只跑赋值，不跑 exec，不会启动渲染器
set -u
cd "$(dirname "$0")/.." || exit 1

fail=0

for f in run-obs.sh run-local.sh launch.sh run.sh tools/*.sh; do
    [ -f "$f" ] || continue
    if ! sh -n "$f" 2>/dev/null; then
        echo "FAIL: $f 有语法错误" >&2
        sh -n "$f" 2>&1 | head -3 >&2
        fail=1
    fi
done

lineno=0
while IFS= read -r line; do
    lineno=$((lineno + 1))
    case "$line" in
        "export "*)
            if ! err=$(sh -c "$line" 2>&1); then
                echo "FAIL: run-obs.sh 第 $lineno 行的赋值无效：" >&2
                echo "      $line" >&2
                echo "      $err" >&2
                fail=1
            fi
            ;;
    esac
done < run-obs.sh

if [ "$fail" -eq 0 ]; then
    echo "PASS: 启动脚本语法、以及全部 export 赋值都正常"
fi
exit "$fail"
