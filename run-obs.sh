#!/bin/sh
# ============================================================
# OBS ASCII 推流启动脚本
#
# 用法：
#   ./run-obs.sh                     # 默认配置启动
#   COLONQ_VIEW=face ./run-obs.sh    # 临时覆盖任何参数
#
# OBS 浏览器源设置：
#   URL:    http://127.0.0.1:8080/?width=385&height=350
#   宽度:   385   高度: 350   帧率: 24
#   不要勾选「源不可见时关闭」
#
# 不会弹出 colonq 预览窗口（正常现象），画面全部通过浏览器源提供。
# ============================================================
set -e
cd "$(dirname "$0")"

# ---------- 推流服务 ----------
export COLONQ_SERVE="${COLONQ_SERVE:-8080}"        # HTTP+SSE 推流端口
export COLONQ_NO_TERM=1                            # 不向终端输出 ANSI（推流模式）
export COLONQ_PREVIEW=0                            # 隐藏 SDL 预览窗口（推流不依赖可见窗口）
export COLONQ_SIZE="${COLONQ_SIZE:-385x350}"       # 终端像素尺寸提示（OBS 源为 385×350）

# ---------- 显示 ----------
export COLONQ_PROFILE="${COLONQ_PROFILE:-1}"       # 每60帧输出分层耗时到日志
export COLONQ_BGCOLOR="${COLONQ_BGCOLOR:-000000}"  # 背景色（RRGGBB）；ffffff=白
export COLONQ_GRID="${COLONQ_GRID:-128x64}"        # 内部渲染网格（也是推流精度）

# ---------- ASCII 字符 ----------
# 开关类变量（COLONQ_PARTS / COLONQ_IDLE / COLONQ_FIT / COLONQ_PROFILE 等）统一取值：
#   1 / true / on / yes  = 开
#   0 / false / off / no = 关
#   未设置或空字符串     = 用程序默认值（不再因为"设了空值"而意外打开）
# 默认填充词（人物身体内部循环显示的字符）
export COLONQ_WORD="${COLONQ_WORD:-lcolonq}"
# 按部位分字符表（需配合 COLONQ_PARTS=1 启用）：
#   部位编号：1=脸/皮肤  2=头发  3=眼睛  4=衣服  9=其他
#   没写进表的部位回落到 COLONQ_WORD
#   示例：COLONQ_PART_WORDS="1=.,2=~,3=#,4=lcolonq"
# 只有显式给了非空值才导出；关闭时干脆不设这个变量
if [ -n "${COLONQ_PARTS:-}" ]; then
    export COLONQ_PARTS
fi
# export COLONQ_PART_WORDS="${COLONQ_PART_WORDS:-2=#,2=@}"

# ---------- 面捕姿态校准（实测确认的组合） ----------
export COLONQ_FLIP="${COLONQ_FLIP:-0}"                     # 组合翻转：0=无 1=Z180 2=Y180 3=X180 4=Z+Y 5=X+Y 6=X+Z
export COLONQ_ROT_OFFSET="${COLONQ_ROT_OFFSET:-0,-20,360}" # 三轴静态偏移（度）：roll,pitch,yaw
export COLONQ_QSIGN="${COLONQ_QSIGN:-1,1,1}"               # 四元数三分量符号 x,y,z
export COLONQ_PITCH_FLIP="${COLONQ_PITCH_FLIP:-1}"         # 俯仰方向反转（点头反向时设 1）
export COLONQ_TRACK_GAIN="${COLONQ_TRACK_GAIN:-1}"         # 头部旋转幅度增益（1=原始，2~3=明显放大）
export COLONQ_EXPR_GAIN="${COLONQ_EXPR_GAIN:-1}"           # 眨眼/张嘴幅度增益
# 面捕 UDP 绑定。**默认收全网卡**：本项目的面捕（OpenSeeFace）跑在局域网里另一台
# 机器上（见 UBUNTU_FACECAP.md），绑 127.0.0.1 会完全收不到任何数据。
# 只有把面捕和渲染放在同一台机器上时才改成 127.0.0.1:11573；
# 共享/不可信网络里建议用防火墙只放行那台面捕机的 IP（UDP 11573）。
export COLONQ_TRACK_BIND="${COLONQ_TRACK_BIND:-0.0.0.0:11573}"

# ---------- 描边发光 ----------
# export COLONQ_OUTLINE="${COLONQ_OUTLINE:-0.010}"                    # 描边宽度（世界单位）
# export COLONQ_OUTLINE_COLOR="${COLONQ_OUTLINE_COLOR:-0.2,0.9,1.0}"  # 描边颜色（青色）
# export COLONQ_OUTLINE_GLOW="${COLONQ_OUTLINE_GLOW:-3.0}"            # 发光强度（0=普通描边）

# ---------- 动画与调试 ----------
export COLONQ_IDLE="${COLONQ_IDLE:1}"             # 待机动画（面捕离线时的摆动）；0=关
export COLONQ_TERM_EVERY="${COLONQ_TERM_EVERY:-1}" # 终端刷新间隔帧数（推流模式无影响）
export COLONQ_SMOOTH="${COLONQ_SMOOTH:-0.5}"       # 面捕平滑系数（0~0.95，越大越平滑）
# COLONQ_VIEW=head|face|bust|full                   # 取景预设（默认 head）
# COLONQ_CAM="x,y,z" / COLONQ_LOOK="x,y,z"          # 手动相机/视线
# COLONQ_SKIN_TINT / COLONQ_SKIN_SAT                # 皮肤染色
# COLONQ_TEX_MAX=512                                # 贴图降采样（省内存）
# COLONQ_TRACK_DEBUG=1                              # 打印面捕原始数据
# COLONQ_DUMP_PNG=1 / COLONQ_DUMP_AT=N              # 存帧调试

# Keep model, grid, camera and color settings inherited from the user's environment.
exec ./run-local.sh "$@"
