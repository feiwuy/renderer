#!/usr/bin/env python3
"""检查推流画面里到底有没有内容。

专门用来抓"渲染进程正常、帧也在发，但画面全黑/全空格"这类故障
（例：异步回读没就绪，像素一直是 0，SSE 发的全是空白格子）。
光比对静态帧的哈希是抓不到的——静态画面下第 0 帧和第 N 帧一样。

用法：
    python3 tools/check_visible.py                      # 默认 127.0.0.1:8080
    python3 tools/check_visible.py --port 8101 --min-cells 200

退出码：0 = 有内容；1 = 内容过少（很可能全黑）；2 = 连不上或没收到帧
"""
import argparse
import json
import sys
import urllib.error
import urllib.request


def first_frame(url):
    with urllib.request.urlopen(url.rstrip('/') + '/stream', timeout=15) as response:
        for raw in response:
            line = raw.decode('utf-8', 'replace')
            if line.startswith('data: '):
                return json.loads(line[6:])
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--url', default='http://127.0.0.1:8080')
    parser.add_argument('--port', type=int, help='只给端口时等价于 --url http://127.0.0.1:<port>')
    parser.add_argument('--min-cells', type=int, default=200,
                        help='非空格子少于这个数就判定为"没有内容"（默认 200）')
    args = parser.parse_args()
    if args.port:
        args.url = f'http://127.0.0.1:{args.port}'

    try:
        frame = first_frame(args.url)
    except (OSError, ValueError, urllib.error.URLError) as error:
        print(f'连不上 {args.url}：{error}', file=sys.stderr)
        return 2

    if frame is None:
        print('没收到任何帧（renderer 可能没启动或 COLONQ_SERVE 端口不对）', file=sys.stderr)
        return 2

    cells = frame.get('cells', [])
    glyphs = {}
    for _index, glyph, _color in cells:
        char = chr(glyph)
        glyphs[char] = glyphs.get(char, 0) + 1

    print(f"帧 seq={frame['seq']} 网格={frame['w']}x{frame['h']} "
          f"非空格子={len(cells)} 调色板={len(frame.get('palette', []))}")
    if glyphs:
        top = sorted(glyphs.items(), key=lambda pair: -pair[1])[:8]
        print('最常见的字符: ' + '  '.join(f'{c!r}×{n}' for c, n in top))

    if len(cells) < args.min_cells:
        print(f'→ 内容过少（< {args.min_cells}），画面很可能是黑的', file=sys.stderr)
        return 1
    print('→ 有内容，画面正常')
    return 0


if __name__ == '__main__':
    sys.exit(main())
