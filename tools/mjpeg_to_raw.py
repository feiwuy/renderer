#!/usr/bin/env python3
"""MJPEG → raw RGB 转发器：把手机摄像头的 MJPEG 流转成 facetracker.py --raw-rgb 模式
需要的标准输入格式（每帧固定 width*height*3 字节，无任何封装）。

用法（和 facetracker 管道使用）：
    python3 mjpeg_to_raw.py --url http://192.168.1.210:4747/video \
        --width 640 --height 360 --fps 24 \
    | python3 facetracker.py --raw-rgb 1 -W 640 -H 360 -F 24 --faces 1 -m 2 \
        -i <arch-ip> -p 11573 --silent 1

依赖：仅 OpenCV（venv 里已装）。无额外下载。

关于延迟（重要）：
    MJPEG 源是"推流"，OpenCV 的 VideoCapture 会自己缓冲若干帧。如果消费者比生产者慢，
    旧帧就在缓冲里越堆越多，表现出来就是"刚启动还行，跑一会儿延迟越来越大"。
    所以这里用一个读线程**持续读、只保留最新一帧**，主循环按目标帧率取用，
    堆积的旧帧直接丢弃（会定期打印丢弃数，正常应该是每秒丢 6 帧左右，
    即源 30fps、输出 24fps 的差值）。
"""
import argparse
import sys
import threading
import time

import cv2


class LatestFrame:
    """后台线程不断读流，只保留最新一帧；取用方永远不会拿到积压的旧帧。"""

    def __init__(self, url, candidates):
        self.url = url
        self.candidates = candidates
        self.lock = threading.Lock()
        self.frame = None          # 最新一帧（BGR）
        self.seq = 0               # 读到第几帧
        self.stopped = False
        self.captured = 0          # 累计读到的帧数
        self.consumed_seq = 0      # 取用方最后拿走的帧号
        self.dropped = 0           # 还没被取用就被新帧覆盖掉的帧数（真正的积压丢弃）
        self.reconnects = 0
        self.thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self.thread.start()
        return self

    def stop(self):
        self.stopped = True

    def _open(self):
        for url in self.candidates:
            cap = cv2.VideoCapture(url)
            # 支持的 backend 上把缓冲压到 1 帧，进一步减少堆积
            try:
                cap.set(cv2.CAP_PROP_BUFFERSIZE, 1)
            except Exception:
                pass
            deadline = time.time() + 8
            while time.time() < deadline:
                ok, _ = cap.read()
                if ok:
                    print(f'source ok: {url}', file=sys.stderr)
                    return cap
                time.sleep(0.1)
            cap.release()
        return None

    def _run(self):
        cap = self._open()
        if cap is None:
            print('FAIL: 所有候选 URL 都读不出帧。用浏览器/VLC 确认手机流可用。', file=sys.stderr)
            self.stopped = True
            return
        while not self.stopped:
            ok, frame = cap.read()
            if not ok:
                # 流偶发中断：重连，而不是退出（延迟优先，先丢掉缓冲）
                cap.release()
                self.reconnects += 1
                time.sleep(0.5)
                cap = self._open()
                if cap is None:
                    self.stopped = True
                    break
                continue
            with self.lock:
                # 上一帧还没被取用就被这一帧覆盖 = 真的丢掉了一帧旧数据
                if self.seq != self.consumed_seq:
                    self.dropped += 1
                self.frame = frame
                self.captured += 1
                self.seq += 1
        if cap is not None:
            cap.release()

    def latest(self):
        """返回 (seq, frame)；还没有帧时返回 (0, None)。取走后记下帧号，供统计丢帧。"""
        with self.lock:
            self.consumed_seq = self.seq
            return self.seq, self.frame


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--url', required=True, help='MJPEG 流地址，例如 http://phone-ip:4747/video')
    parser.add_argument('--width', type=int, default=640)
    parser.add_argument('--height', type=int, default=360)
    parser.add_argument('--fps', type=float, default=24.0, help='输出节流帧率')
    parser.add_argument('--report-seconds', type=float, default=10.0,
                        help='每隔多少秒打印一次统计（0 = 不打印）')
    parser.add_argument('--threads', type=int, default=1,
                        help='OpenCV 内部线程数（0 = 用 OpenCV 默认值，通常等于核数）。'
                             '默认 1：解码/resize 不铺满多核，机器更安静、风扇更小；'
                             '想榨速度再设 0')
    args = parser.parse_args()

    if args.threads > 0:
        # OpenCV 默认会用满所有核心做 resize/cvtColor 这类并行循环，
        # 在小机器上表现为「核心瞬间全开 → 风扇转速一直不降」。
        cv2.setNumThreads(args.threads)

    # 多个候选 URL，逐个尝试直到能读出帧
    candidates = [args.url]
    base = args.url.split('?')[0].rstrip('/')
    for extra in ('/video', '/mjpegfeed?{}x{}'.format(args.width, args.height), '/cam.mjpeg'):
        if base + extra not in candidates:
            candidates.append(base + extra)

    print(f'OpenCV 线程数: {"默认（按核数）" if args.threads <= 0 else args.threads}', file=sys.stderr)
    source = LatestFrame(args.url, candidates).start()

    frame_duration = 1.0 / max(args.fps, 0.1)
    next_due = time.perf_counter()
    written = 0
    last_seq = 0
    last_report = time.perf_counter()
    last_captured = last_dropped = last_written = 0
    try:
        while True:
            next_due += frame_duration
            wait = next_due - time.perf_counter()
            if wait > 0:
                time.sleep(wait)
            else:
                # 落后太多（例如下游卡顿）就重置节拍，避免补帧堆积
                next_due = time.perf_counter()

            if source.stopped and source.seq == 0:
                print('FAIL: 视频源无法读取，退出', file=sys.stderr)
                return 1

            seq, frame = source.latest()
            if frame is None:
                continue
            if seq == last_seq:
                # 这一轮没有新帧（源比目标帧率慢）：不重复发送同一帧
                continue
            last_seq = seq

            if frame.shape[1] != args.width or frame.shape[0] != args.height:
                frame = cv2.resize(frame, (args.width, args.height), interpolation=cv2.INTER_AREA)
            frame = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
            sys.stdout.buffer.write(frame.tobytes())
            sys.stdout.buffer.flush()
            written += 1

            if args.report_seconds > 0 and time.perf_counter() - last_report >= args.report_seconds:
                elapsed = time.perf_counter() - last_report
                last_report = time.perf_counter()
                with source.lock:
                    captured, dropped, reconnects = source.captured, source.dropped, source.reconnects
                # 用「本轮增量」而不是累计值算速率：累计值除以单轮时长会越算越大，
                # 看起来像源帧率在爬升（早期版本的统计 bug）
                d_captured, d_dropped, d_written = captured - last_captured, dropped - last_dropped, written - last_written
                last_captured, last_dropped, last_written = captured, dropped, written
                print(f'本轮输出 {d_written} 帧（{d_written / elapsed:.1f} fps）；'
                      f'源 {d_captured / elapsed:.1f} fps；'
                      f'丢弃旧帧 {d_dropped} 个（{d_dropped / elapsed:.1f}/s，累计 {dropped}）；'
                      f'重连 {reconnects} 次',
                      file=sys.stderr)
                sys.stderr.flush()
    except KeyboardInterrupt:
        return 0
    except BrokenPipeError:
        # 下游（head 管道测试 / facetracker 退出）关闭了管道：正常结束，不是错误
        print(f'stream closed by downstream after {written} frames', file=sys.stderr)
        return 0
    finally:
        source.stop()


if __name__ == '__main__':
    sys.exit(main())
