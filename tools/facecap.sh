#!/bin/bash
# 面捕启动脚本（Ubuntu Server）
# 用法：~/facecap.sh
# 手机 IP 变了就改下面的 URL。建议在路由器里给手机绑定静态 IP。
pkill -f mjpeg_to_raw 2>/dev/null
pkill -f facetracker.py 2>/dev/null
sleep 1
cd ~/OpenSeeFace
~/osf-env/bin/python mjpeg_to_raw.py --url http://192.168.1.210:4747/video --width 640 --height 360 --fps 24 \
  | ~/osf-env/bin/python facetracker.py --raw-rgb 1 -W 640 -H 360 -F 24 --faces 1 -m 2 \
    -i 192.168.1.100 -p 11573 --silent 1
