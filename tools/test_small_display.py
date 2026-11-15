#!/usr/bin/env python3
"""Validate small-terminal output, headless streaming, and a real Firefox viewport.
No installation or downloads. All child processes are scoped to this test and stopped on exit.
"""
import argparse
import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import shutil
import socket
import struct
import subprocess
import tempfile
import termios
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ROOT / 'validation-obs'
BIN = ROOT / 'target/release/colonq'


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def stop(process):
    if process and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def frame(port):
    with urllib.request.urlopen(f'http://127.0.0.1:{port}/snapshot', timeout=2) as r:
        return json.load(r)


def wait_frame(port, process, seconds=15):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        if process.poll() is not None:
            raise RuntimeError(f'renderer exited {process.returncode}')
        try:
            last = frame(port)
            if last['cells']:
                return last
        except (OSError, ValueError):
            pass
        time.sleep(0.1)
    raise TimeoutError(f'no rendered frame: {last}')


def environment(port):
    env = os.environ.copy()
    for key in list(env):
        if key.startswith('COLONQ_'):
            del env[key]
    env.update(COLONQ_SERVE=str(port), COLONQ_ASSETS=str(ROOT/'assets'), COLONQ_NO_MPV='1',
               COLONQ_PROFILE='1', COLONQ_FPS='24', COLONQ_GRID='128x64',
               COLONQ_TEX_MAX='512', COLONQ_IDLE='0')
    return env


class Marionette:
    def __init__(self, port, process):
        end = time.monotonic() + 20
        while True:
            try:
                self.socket = socket.create_connection(('127.0.0.1', port), timeout=2)
                break
            except OSError:
                if process.poll() is not None or time.monotonic() > end:
                    raise RuntimeError('Firefox Marionette did not start')
                time.sleep(0.2)
        self.socket.settimeout(15)
        self.receive()
        self.ident = 0
        self.command('WebDriver:NewSession', {'capabilities': {'alwaysMatch': {'acceptInsecureCerts': True}}})

    def receive(self):
        length = b''
        while not length.endswith(b':'):
            ch = self.socket.recv(1)
            if not ch:
                raise EOFError('Firefox disconnected')
            length += ch
        size = int(length[:-1])
        body = b''
        while len(body) < size:
            chunk = self.socket.recv(size-len(body))
            if not chunk:
                raise EOFError('Firefox disconnected')
            body += chunk
        return json.loads(body)

    def command(self, name, args=None):
        self.ident += 1
        packet = json.dumps([0, self.ident, name, args or {}]).encode()
        self.socket.sendall(str(len(packet)).encode()+b':'+packet)
        while True:
            reply = self.receive()
            if isinstance(reply, list) and reply[0] == 1 and reply[1] == self.ident:
                if reply[2]:
                    raise RuntimeError(f'{name}: {reply[2]}')
                result = reply[3]
                return result.get('value', result) if isinstance(result, dict) else result

    def execute(self, script):
        return self.command('WebDriver:ExecuteScript', {'script': script, 'args': [], 'newSandbox': False})

    def close(self):
        try:
            self.command('WebDriver:DeleteSession')
        except (OSError, RuntimeError, EOFError):
            pass
        self.socket.close()


def test_browser(url, profile_dir, width=385, height=350):
    firefox = shutil.which('firefox')
    if not firefox:
        print('SKIP: Firefox not installed; no browser was downloaded')
        return None
    port = free_port()
    profile = profile_dir / 'firefox-profile'
    profile.mkdir()
    (profile/'user.js').write_text(f'user_pref("marionette.port", {port});\nuser_pref("browser.shell.checkDefaultBrowser", false);\n')
    browser = None
    client = None
    with (ARTIFACTS/'firefox.log').open('wb') as log:
        try:
            browser = subprocess.Popen([firefox, '--headless', '--no-remote', '--profile', str(profile), '--marionette'],
                                       stdout=log, stderr=log)
            client = Marionette(port, browser)
            client.command('WebDriver:SetWindowRect', {'width': 500, 'height': 500})
            small_url = url + f'/?width={width}&height={height}'
            client.command('WebDriver:Navigate', {'url': small_url})
            # Firefox enforces a minimum top-level window width. Use a same-origin iframe
            # to exercise the viewer in a genuine 329×295 viewport, not a resized screenshot.
            client.execute(f"""
                const probe = document.createElement('iframe');
                probe.id = 'probe-viewport';
                probe.width = '{width}'; probe.height = '{height}';
                probe.style.cssText = 'position:fixed;left:0;top:0;width:{width}px;height:{height}px;border:0';
                probe.src = location.href;
                document.body.appendChild(probe);
                return true;
            """)
            state_script = "return document.getElementById('probe-viewport').contentWindow.__COLONQ_STATE__ || null;"
            deadline = time.monotonic() + 10
            state = None
            while time.monotonic() < deadline:
                state = client.execute(state_script)
                if state and state.get('paintCount', 0) > 2:
                    break
                time.sleep(0.1)
            assert state and state.get('paintCount', 0) > 2, state
            assert state['grid'] == [128, 64], state
            assert state['viewport'] == [width, height], state
            box = state['layout']
            assert abs(box['cellHeight'] / box['cellWidth'] - 2) < 1e-8, box
            assert box['contentWidth'] <= width+0.0001 and box['contentHeight'] <= height+0.0001, box
            assert box['left'] >= 0 and box['top'] >= 0, box
            first_seq = state['seq']
            time.sleep(1)
            state = client.execute(state_script)
            assert state['seq'] > first_seq, state
            element = client.execute("return document.getElementById('probe-viewport');")
            ident = element.get('element-6066-11e4-a52e-4f735466cecf') or element.get('ELEMENT')
            screenshot = client.command('WebDriver:TakeScreenshot', {'id': ident, 'full': False})
            (ARTIFACTS/f'browser-{width}x{height}.png').write_bytes(base64.b64decode(screenshot))
            (ARTIFACTS/f'browser-{width}x{height}-state.json').write_text(json.dumps(state, ensure_ascii=False, indent=2))
            print('PASS: Firefox 实际绘制，接收fps=',round(state['receivedFps'],2),'绘制ms=',round(state['drawMs'],2),'viewport=', state['viewport'], 'grid=', state['grid'],
                  'content=', [box['contentWidth'], box['contentHeight']], '持续推帧')
            return state
        finally:
            if client:
                client.close()
            stop(browser)


def drain(master, duration):
    result = bytearray()
    end = time.monotonic() + duration
    while time.monotonic() < end:
        ready, _, _ = select.select([master], [], [], min(0.1, max(0, end-time.monotonic())))
        if ready:
            try:
                result.extend(os.read(master, 65536))
            except OSError:
                break
    return bytes(result)


def test_terminal(work):
    port = free_port()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 21, 47, 329, 295))
    env = environment(port)
    env.update(COLONQ_SIZE='329x295', COLONQ_PIXELS='1')
    process = None
    with (ARTIFACTS/'terminal-stderr.log').open('wb') as log:
        try:
            process = subprocess.Popen([str(BIN)], cwd=work, env=env, stdin=slave, stdout=slave, stderr=log)
            os.close(slave)
            current = wait_frame(port, process)
            capture = drain(master, 1)
            assert (current['w'], current['h']) == (128,64), current
            positions = re.findall(rb'\x1b\[(\d+);1H', capture)
            assert positions and max(map(int, positions)) <= 20
            before = frame(port)['seq']
            # Deliberately don't read the PTY; a full pipe must not block the stream.
            time.sleep(2)
            after = frame(port)['seq']
            assert after - before >= 25, (before, after)
            print('PASS: 329×295 终端(47×21格)输出不超过20行，推流仍128×64；PTY停止读取时继续推帧')
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack('HHHH', 16, 35, 245, 224))
            drain(master, 0.5)
            capture = drain(master, 1)
            complete = capture.split(b'\x1b[?2026h')[-2]
            positions = re.findall(rb'\x1b\[(\d+);1H', complete)
            assert positions and max(map(int, positions)) <= 15
            current = frame(port)
            assert (current['w'], current['h']) == (128,64)
            print('PASS: 终端继续缩小至35×16格，安全输出不超过15行，OBS精度不变')
        finally:
            stop(process)
            os.close(master)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--no-browser', action='store_true')
    args = parser.parse_args()
    ARTIFACTS.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='renderer-small-validation-') as temp:
        work = Path(temp)
        assert work.resolve().parent == Path(tempfile.gettempdir()).resolve() and work.name.startswith('renderer-small-validation-')
        port = free_port()
        env = environment(port)
        env['COLONQ_NO_TERM'] = '1'
        process = None
        try:
            with (ARTIFACTS/'renderer-stderr.log').open('wb') as log:
                process = subprocess.Popen([str(BIN)], env=env, cwd=work, stdin=subprocess.DEVNULL,
                                           stdout=subprocess.DEVNULL, stderr=log)
                current = wait_frame(port, process)
                assert (current['w'], current['h']) == (128,64)
                assert current['cells'] and current['palette']
                a = current['seq']; time.sleep(1); b = frame(port)['seq']
                print('PASS: NO_TERM + 非TTY，128×64 持续推帧，1秒序号增加', b-a)
                assert 12 <= b-a <= 40, (a,b)
                if not args.no_browser:
                    test_browser(f'http://127.0.0.1:{port}', work)
                probe = subprocess.run([os.sys.executable, str(ROOT/'tools/check_stream.py'),
                                        '--url', f'http://127.0.0.1:{port}', '--seconds', '4'], check=True)
                log_text = (work/'colonq.log').read_text(errors='replace')
                assert 'SDL 预览: 隐藏；窗口交换: 禁用' in log_text
                print('PASS: 无可见colonq窗口且不调用swap时仍持续推流')
                (ARTIFACTS/'renderer-profile.log').write_text('\n'.join(line for line in log_text.splitlines()
                    if any(key in line for key in ['profile','SDL 预览','GL 后端'])))
        finally:
            stop(process)
        test_terminal(work)
    print('全部运行验证完成；没有改动用户窗口/OBS设置，也没有留下测试服务器。')


if __name__ == '__main__':
    main()
