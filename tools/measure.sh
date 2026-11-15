#!/bin/bash
# 采集 renderer 的运行占用
# 用法: ./tools/measure.sh [网格] [采样秒数]      例: ./tools/measure.sh 128x96 6
#
# 输出：CPU%、RSS、线程数、终端输出带宽、渲染后端（GPU 还是 llvmpipe 软件渲染）、分层耗时
grid="${1:-128x96}"; dur="${2:-6}"
cd "$(dirname "$0")/.." || exit 1
rm -f colonq.log

DISPLAY="${DISPLAY:-:0}" COLONQ_GRID="$grid" COLONQ_PROFILE=1 \
  setsid script -qec "stty cols 240 rows 120; exec ./target/release/colonq" /dev/null >/dev/null 2>&1 &
sleep 4   # 等模型加载、进入稳定帧循环

bin="$PWD/target/release/colonq"
pid=""
for p in $(pgrep -x colonq); do
    [ "$(readlink -f /proc/$p/exe 2>/dev/null)" = "$bin" ] && pid=$p && break
done
if [ -z "$pid" ]; then echo "启动失败（检查 DISPLAY / assets）"; pkill -x colonq; exit 1; fi

jif()  { awk '{print $14+$15}' /proc/$pid/stat 2>/dev/null; }
wch()  { awk '/^wchar/{print $2}' /proc/$pid/io 2>/dev/null; }
fld()  { awk -v k="$1" '$1==k":"{print $2}' /proc/$pid/status 2>/dev/null; }

c0=$(jif); w0=$(wch)
declare -A t0
for t in /proc/$pid/task/*; do t0[$(basename "$t")]=$(awk '{print $14+$15}' "$t/stat" 2>/dev/null); done

sleep "$dur"

c1=$(jif); w1=$(wch)
rss=$(fld VmRSS); thr=$(fld Threads); hz=$(getconf CLK_TCK)
cpu=$(awk -v a="$c0" -v b="$c1" -v d="$dur" -v hz="$hz" 'BEGIN{printf "%.0f", (b-a)/hz/d*100}')
kbs=$(awk -v a="$w0" -v b="$w1" -v d="$dur" 'BEGIN{printf "%.0f", (b-a)/1024/d}')

# 渲染后端：看映射了哪个 GL 驱动
# 有 llvmpipe / swrast 线程在跑 = 软件渲染（Mesa 的 CPU 光栅化器）
soft=$(ls /proc/$pid/task/*/comm 2>/dev/null | xargs -r -n1 cat 2>/dev/null | grep -cE 'llvmpipe|swrast')
backend="未知"
if [ "${soft:-0}" -gt 0 ]; then backend="llvmpipe 软件渲染($soft 线程)"
elif grep -qa 'libnvidia-glcore' /proc/$pid/maps 2>/dev/null; then backend="NVIDIA 硬件"
elif grep -qa 'radeonsi' /proc/$pid/maps 2>/dev/null; then backend="AMD 硬件(radeonsi)"; fi

# 最忙的线程
top=""
while read -r v id; do
    n=$(cat /proc/$pid/task/$id/comm 2>/dev/null)
    top+=$(awk -v v="$v" -v hz="$hz" -v d="$dur" -v n="$n" 'BEGIN{printf "%s %.0f%% / ", n, v/hz/d*100}')
done < <(for t in /proc/$pid/task/*; do id=$(basename "$t"); c=$(awk '{print $14+$15}' "$t/stat" 2>/dev/null); echo "$((c-${t0[$id]:-0})) $id"; done | sort -rn | head -3)

prof=$(grep 'profile' colonq.log 2>/dev/null | tail -1 | sed -E 's/.*profile //')

printf '网格 %-8s 后端 %-18s CPU %-4s%% 内存 %-5sMB 线程 %-3s 输出 %-4sKB/s\n' \
       "$grid" "$backend" "$cpu" "$((rss/1024))" "$thr" "$kbs"
printf '           线程占用: %s\n' "${top% / }"
printf '           %s\n' "$prof"

pkill -x colonq; sleep 0.5
