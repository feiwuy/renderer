#!/usr/bin/env python3
"""MJPEG → raw RGB 转发器：把手机摄像头的 MJPEG 流转成 facetracker.py --raw-rgb 模式
需要的标准输入格式（每帧固定 width*height*3 字节，无任何封装）。

用法（和 facetracker 管道使用）：
    python3 mjpeg_to_raw.py --url http://192.168.1.210:4747/video \
        --width 640 --height 360 --fps 24 \
    | python3 facetracker.py --raw-rgb 1 -W 640 -H 360 -F 24 --faces 1 -m 2 \
        -i <arch-ip> -p 11573 --silent 1

依赖：仅 OpenCV + numpy（venv 里已装）。无额外下载。
"""
import argparse
import sys
import time

import cv2
import numpy as np


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--url', required=True, help='MJPEG 流地址，例如 http://phone-ip:4747/video')
    parser.add_argument('--width', type=int, default=640)
    parser.add_argument('--height', type=int, default=360)
    parser.add_argument('--fps', type=float, default=24.0, help='输出节流帧率')
    parser.add_argument('--jpeg-quality', type=int, default=90)
    args = parser.parse_args()

    # 多个候选 URL，逐个尝试直到能读出帧
    candidates = [args.url]
    base = args.url.split('?')[0].rstrip('/')
    for extra in ('/video', '/mjpegfeed?{}x{}'.format(args.width, args.height), '/cam.mjpeg'):
        if base + extra not in candidates:
            candidates.append(base + extra)

    cap = None
    for url in candidates:
        cap = cv2.VideoCapture(url)
        # MJPEG 流可能要几秒才出第一帧；用 read 的阻塞特性等
        deadline = time.time() + 8
        ok = False
        while time.time() < deadline:
            ok, _ = cap.read()
            if ok:
                break
            time.sleep(0.1)
        if ok:
            print(f'source ok: {url}', file=sys.stderr)
            break
        cap.release()
    if cap is None or not cap.isOpened():
        print('FAIL: 所有候选 URL 都读不出帧。用浏览器/VLC 确认手机流可用。', file=sys.stderr)
        return 1

    frame_duration = 1.0 / max(args.fps, 0.1)
    last = time.perf_counter()
    written = 0
    try:
        while True:
            # 节流到目标帧率，减轻 Wi-Fi 和下游 facetracker 的压力
            now = time.perf_counter()
            wait = frame_duration - (now - last)
            if wait > 0:
                time.sleep(wait)
            last = time.perf_counter()

            ok, frame = cap.read()
            if not ok:
                # MJPEG 流偶尔丢一帧，重连而不是退出
                cap.release()
                time.sleep(0.5)
                cap = cv2.VideoCapture(candidates[0])
                continue

            if frame.shape[1] != args.width or frame.shape[0] != args.height:
                frame = cv2.resize(frame, (args.width, args.height), interpolation=cv2.INTER_AREA)
            frame = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            sys.stdout.buffer.write(frame.tobytes())
            written += 1
            if written % (args.fps * 10) == 0:
                print(f'forwarded {written} frames', file=sys.stderr)
                sys.stderr.flush()
    except KeyboardInterrupt:
        return 0
    except BrokenPipeError:
        # 下游（head 管道测试 / facetracker 退出）关闭了管道：正常结束，不是错误
        print(f'stream closed by downstream after {written} frames', file=sys.stderr)
        return 0
    finally:
        cap.release()


if __name__ == '__main__':
    sys.exit(main())
