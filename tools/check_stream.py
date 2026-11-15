#!/usr/bin/env python3
"""Read-only timing probe for an existing local ASCII stream; does not start/stop renderer.
Usage: python3 tools/check_stream.py [--url http://127.0.0.1:8080] [--seconds 6]
"""
import argparse
import json
import math
import socket
import time
import urllib.error
import urllib.request


def measure(url, seconds):
    stamps = []
    started = time.monotonic()
    try:
        with urllib.request.urlopen(url.rstrip('/')+'/stream', timeout=5) as response:
            while time.monotonic() - started < seconds:
                line = response.readline()
                if not line:
                    break
                if not line.startswith(b'data: '):
                    continue
                data = json.loads(line[6:])
                now = time.monotonic()
                if not stamps or data['seq'] != stamps[-1][1]:
                    age = max(0, time.time()*1000-data['producedAt']) if data.get('producedAt') else None
                    stamps.append((now, data['seq'], data.get('producedAt'), age))
    except (OSError, ValueError, urllib.error.URLError, socket.timeout) as error:
        print('推流连接未持续收到数据:', error)
    if len(stamps) < 2:
        print('样本不足。确认 renderer 仍在运行、8080端口可用，以及 OBS URL 是浏览器源而非屏幕采集。')
        return 1
    elapsed = stamps[-1][0]-stamps[0][0]
    received = (len(stamps)-1)/elapsed
    skipped = sum(max(0, b[1]-a[1]-1) for a,b in zip(stamps,stamps[1:]))
    gaps = sorted((b[0]-a[0])*1000 for a,b in zip(stamps,stamps[1:]))
    p95 = gaps[min(len(gaps)-1, math.ceil(len(gaps)*0.95)-1)]
    ages = [item[3] for item in stamps if item[3] is not None]
    print(f'接收 {received:.2f} fps；帧间隔 P95 {p95:.1f} ms；最大间隔 {max(gaps):.1f} ms；跳过序号 {skipped}')
    if stamps[0][2] and stamps[-1][2] and stamps[-1][2] > stamps[0][2]:
        generated = (stamps[-1][1]-stamps[0][1])*1000/(stamps[-1][2]-stamps[0][2])
        print(f'ASCII发布约 {generated:.2f} fps；平均帧龄 {sum(ages)/len(ages):.1f} ms；最大帧龄 {max(ages):.1f} ms')
    print('此工具不测 OBS 合成帧率。若这里稳定约24而OBS仍卡，用URL加 &debug=1 查看浏览器接收/绘制帧率。')
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', default='http://127.0.0.1:8080')
    parser.add_argument('--seconds', type=float, default=6)
    args = parser.parse_args()
    if not (0 < args.seconds <= 60):
        parser.error('--seconds must be between 0 and 60')
    return measure(args.url, args.seconds)


if __name__ == '__main__':
    raise SystemExit(main())
