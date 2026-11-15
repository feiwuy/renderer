#!/usr/bin/env python3
"""最小 VRM 0.x -> VRM 1.0 转换器（只为本项目 lcolonq/renderer 服务）

做法：GLB 的二进制块（网格/骨骼权重/贴图）完全不动，只把 JSON 里的
`extensions.VRM` 重写成 `extensions.VRMC_vrm`，因为 renderer 只读这些字段：

    VRMC_vrm.humanoid.humanBones  -> 每根骨骼的 node 索引
    VRMC_vrm.expressions.preset   -> 表情的 morphTargetBinds

注意：这不是完整的 VRM 1.0 转换（meta / springBone / MToon 材质参数都没迁），
只保证本渲染器能正确加载。要让别的软件也吃得下，请用 Blender + VRM Add-on 转。

用法：
    python3 vrm0_to_vrm1_min.py 输入.vrm 输出.vrm
"""

import json
import struct
import sys

# VRM 0.x 的 presetName -> VRM 1.0 的 preset 名称
PRESET_MAP = {
    "neutral": "neutral",
    "a": "aa",
    "i": "ih",
    "u": "ou",
    "e": "ee",
    "o": "oh",
    "blink": "blink",
    "blink_l": "blinkLeft",
    "blink_r": "blinkRight",
    "angry": "angry",
    "fun": "relaxed",
    "joy": "happy",
    "sorrow": "sad",
    "unknown": "surprised",
}

GLB_MAGIC = 0x46546C67
CHUNK_JSON = 0x4E4F534A
CHUNK_BIN = 0x004E4942


def read_glb(path):
    data = open(path, "rb").read()
    magic, version, length = struct.unpack("<III", data[:12])
    if magic != GLB_MAGIC:
        raise SystemExit("不是 GLB/VRM 文件")
    off, gltf, binary = 12, None, b""
    while off < length:
        clen, ctype = struct.unpack("<II", data[off:off + 8])
        off += 8
        chunk = data[off:off + clen]
        off += clen
        if ctype == CHUNK_JSON:
            gltf = json.loads(chunk)
        elif ctype == CHUNK_BIN:
            binary = chunk
    return gltf, binary


def write_glb(path, gltf, binary):
    js = json.dumps(gltf, separators=(",", ":")).encode("utf-8")
    js += b" " * ((4 - len(js) % 4) % 4)          # JSON chunk 用空格补齐
    bin_pad = b"\0" * ((4 - len(binary) % 4) % 4)
    total = 12 + 8 + len(js) + (8 + len(binary) + len(bin_pad) if binary else 0)
    with open(path, "wb") as f:
        f.write(struct.pack("<III", GLB_MAGIC, 2, total))
        f.write(struct.pack("<II", len(js), CHUNK_JSON))
        f.write(js)
        if binary:
            f.write(struct.pack("<II", len(binary) + len(bin_pad), CHUNK_BIN))
            f.write(binary)
            f.write(bin_pad)


def mesh_to_node_table(gltf):
    """mesh 索引 -> 引用它的第一个 node 索引（VRM 1.0 的 morphTargetBind 要用 node）"""
    table = {}
    for i, node in enumerate(gltf.get("nodes", [])):
        if "mesh" in node:
            table.setdefault(node["mesh"], i)
    return table


def convert(gltf):
    ext = gltf.setdefault("extensions", {})
    vrm0 = ext.get("VRM")
    if vrm0 is None:
        if "VRMC_vrm" in ext:
            raise SystemExit("这个文件已经是 VRM 1.0，不用转")
        raise SystemExit("没有 VRM 扩展，不是 VRM 模型")

    # ---- humanoid：数组 [{bone,node}] -> 映射 {name:{node}} ----
    bones = {}
    for b in vrm0.get("humanoid", {}).get("humanBones", []):
        name = b.get("bone")
        if name and "node" in b:
            bones[name] = {"node": b["node"]}

    # ---- expressions：blendShapeGroups -> preset ----
    mesh2node = mesh_to_node_table(gltf)
    preset, skipped = {}, []
    for grp in vrm0.get("blendShapeMaster", {}).get("blendShapeGroups", []):
        name = PRESET_MAP.get(grp.get("presetName") or "")
        binds = []
        for bind in grp.get("binds", []):
            node = mesh2node.get(bind.get("mesh"))
            if node is None:
                continue
            w = bind.get("weight", 1.0)
            if w > 1.5:      # VRM 0.x 用 0~100，VRM 1.0 用 0~1
                w = w / 100.0
            binds.append({
                "node": node,
                "index": bind.get("index", 0),
                "weight": w,
            })
        if name is None or not binds:
            skipped.append(grp.get("presetName") or grp.get("name"))
            continue
        preset.setdefault(name, {"morphTargetBinds": binds, "isBinary": False})

    meta0 = vrm0.get("meta", {}) or {}
    vrmc = {
        "specVersion": "1.0",
        "meta": {
            "name": meta0.get("title") or "",
            "authors": [a for a in [meta0.get("author")] if a],
            "licenseUrl": "https://vrm.dev/licenses/1.0/",
        },
        "humanoid": {"humanBones": bones},
        "expressions": {"preset": preset},
    }
    ext["VRMC_vrm"] = vrmc

    return {
        "bones": len(bones),
        "presets": sorted(preset.keys()),
        "skipped": skipped,
        "materials_with_vrm0": len(vrm0.get("materialProperties", [])),
    }


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    src, dst = sys.argv[1], sys.argv[2]
    gltf, binary = read_glb(src)
    info = convert(gltf)
    write_glb(dst, gltf, binary)
    print("转换完成:", dst)
    print("  骨骼:", info["bones"])
    print("  表情 preset:", info["presets"])
    if info["skipped"]:
        print("  跳过的表情组:", info["skipped"])
    print("  提示: VRM 0.x 里有 %d 份 MToon 材质参数未迁移（本渲染器不读）" % info["materials_with_vrm0"])


if __name__ == "__main__":
    main()
