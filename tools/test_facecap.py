#!/usr/bin/env python3
"""面捕 UDP 测试包发送器（fish 里直接 `python3 tools/test_facecap.py [目标IP]` 运行）
发送 4 秒标准 1785 字节单脸包：眼睛睁开、单位四元数、嘴巴张开 0.9。
"""
import socket
import struct
import sys
import time
import math

target_ip = sys.argv[1] if len(sys.argv) > 1 else '127.0.0.1'
target_port = int(sys.argv[2]) if len(sys.argv) > 2 else 11573

import math as _m

def quat_euler_xyz(roll, pitch, yaw):
    """ZYX 内旋/XYZ 外旋与 glam::Quat::from_euler(XYZ,...) 一致的四元数"""
    cr, cp, cy = _m.cos(roll/2), _m.cos(pitch/2), _m.cos(yaw/2)
    sr, sp, sy = _m.sin(roll/2), _m.sin(pitch/2), _m.sin(yaw/2)
    return (sr*cp*cy - cr*sp*sy,   # x
            cr*sp*cy + sr*cp*sy,   # y
            cr*cp*sy - sr*sp*cy,   # z
            cr*cp*cy + sr*sp*sy)   # w

# 上游补偿是 from_xyzw(x,-y,-z,w) × from_euler(XYZ, PI, 0, PI/2)
# 要让补偿后 ≈ 正脸，发送值取补偿的逆：from_euler 的逆的逆变换回来
comp = quat_euler_xyz(math.pi, 0.0, math.pi/2)          # 补偿四元数 (x,y,z,w)
inv = (comp[0], comp[1], comp[2], -comp[3])             # 共轭 = 逆（单位四元数）
# 发送端会被再次翻转 y,z：from_xyzw(x,-y,-z,w)，所以实际要发的是
# 使 (x,-y,-z,w) == inv 的 (x,y,z,w)
send = (inv[0], -inv[1], -inv[2], inv[3])

p = bytearray(1785)
struct.pack_into('<ff', p, 20, 0.2, 0.2)                     # 两眼睁开（blink = 1-0.2 = 0.8）
struct.pack_into('<ffff', p, 33, *send)                      # 补偿后≈正脸
struct.pack_into('<f', p, 1777, 0.9)                         # 嘴巴张开 0.9
print(f'发送四元数: {send}（补偿后≈正脸）')

s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
end = time.time() + 4
n = 0
while time.time() < end:
    s.sendto(bytes(p), (target_ip, target_port))
    n += 1
    time.sleep(0.04)
print(f'已向 {target_ip}:{target_port} 发送 {n} 个测试包（4秒）')
