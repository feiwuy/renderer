#!/usr/bin/env python3
"""旋转方向校准器：轮流发送不同补偿变体，看 OBS 里哪个让头部回正。
每档 5 秒，终端提示当前档位。找到正确档位后，把档号告诉我或填进 COLONQ_YAW_FIX。
"""
import socket
import struct
import sys
import time
import math

target_ip = sys.argv[1] if len(sys.argv) > 1 else '127.0.0.1'
target_port = int(sys.argv[2]) if len(sys.argv) > 2 else 11573

def quat_euler_xyz(roll, pitch, yaw):
    cr, cp, cy = math.cos(roll/2), math.cos(pitch/2), math.cos(yaw/2)
    sr, sp, sy = math.sin(roll/2), math.sin(pitch/2), math.sin(yaw/2)
    return (sr*cp*cy - cr*sp*sy, cr*sp*cy + sr*cp*sy,
            cr*cp*sy - sr*sp*cy, cr*cp*cy + sr*sp*sy)

def qmul(a, b):
    ax, ay, az, aw = a; bx, by, bz, bw = b
    return (aw*bx + ax*bw + ay*bz - az*by,
            aw*by - ax*bz + ay*bw + az*bx,
            aw*bz + ax*by - ay*bx + az*bw,
            aw*bw - ax*bx - ay*by - az*bz)

def conjugate(q): return (q[0], q[1], q[2], -q[3])

# 原始补偿：from_xyzw(x,-y,-z,w) × from_euler(XYZ, PI, 0, PI/2)
COMP = quat_euler_xyz(math.pi, 0.0, math.pi/2)
FLIP_Y = quat_euler_xyz(0.0, math.pi, 0.0)   # 绕 Y 轴 180°：掉头
FLIP_X = quat_euler_xyz(math.pi, 0.0, 0.0)   # 绕 X 轴 180°：前滚翻
FLIP_Z = quat_euler_xyz(0.0, 0.0, math.pi)   # 绕 Z 轴 180°：左右翻转

# 档位：发送包里的四元数 q_send 满足 from_xyzw(qx,-qy,-qz,qw) × COMP × EXTRA = 期望姿态
# 期望"正脸"= 恒等 → q_send = inverse(COMP × EXTRA) 的翻转前形式
variants = [
    ('0: 原始（现状，头倒置）', None),
    ('1: 乘 Y轴180°（掉头，最常见的修复）', FLIP_Y),
    ('2: 乘 X轴180°（前滚翻）', FLIP_X),
    ('3: 乘 Z轴180°（左右翻转）', FLIP_Z),
]

def make_packet(quat_xyzw):
    p = bytearray(1785)
    struct.pack_into('<ff', p, 20, 0.2, 0.2)
    struct.pack_into('<ffff', p, 33, *quat_xyzw)
    struct.pack_into('<f', p, 1777, 0.0)
    return bytes(p)

s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
try:
    for label, extra in variants:
        comp = quat_euler_xyz(math.pi, 0.0, math.pi/2)
        if extra is None:
            send = comp
        else:
            send = conjugate(qmul(comp, extra))   # 补偿×附加 的逆，翻转 y/z 后发送
        # 发送端还会做 from_xyzw(x,-y,-z,w) 翻转，所以包里存的 (x,y,z,w) 要先翻转回来
        packet_q = (send[0], -send[1], -send[2], send[3])
        print(f'\n>>> 档位 {label}  （5 秒，观察头部朝向）', flush=True)
        deadline = time.time() + 5
        n = 0
        while time.time() < deadline:
            s.sendto(make_packet(packet_q), (target_ip, target_port))
            n += 1
            time.sleep(0.04)
        print(f'    已发 {n} 包', flush=True)
        time.sleep(0.5)
    print('\n全部档位播完。哪个档位让头部回正？把档号告诉我。', flush=True)
finally:
    s.close()
