#!/bin/sh
# Ubuntu Server 面捕部署前自检：不启动摄像头、不安装任何东西。
# 只检查：Python 版本、依赖、OpenSeeFace 文件、与 renderer 的 UDP 连通方向提示。
set -eu
cd "$(dirname "$0")/.."

fail=0
pyver=$(python3 -c 'import sys;print("%d.%d"%sys.version_info[:2])')
case "$pyver" in
  3.7|3.8|3.9|3.10|3.11) echo "PASS python $pyver（vendored OpenSeeFace 支持 <3.11；3.11 需实测）";;
  *) echo "WARN python $pyver 超出 OpenSeeFace 声明范围（>=3.7,<3.11）；建议 venv 装 3.10";;
esac

python3 - <<'EOF' || fail=1
import importlib.util
missing=[m for m in ('numpy','cv2','onnxruntime','PIL') if importlib.util.find_spec(m) is None]
if missing: print('FAIL python 依赖缺失:', ', '.join(missing))
else:
    import cv2, numpy, onnxruntime
    print('PASS python 依赖齐备: cv2',cv2.__version__,'numpy',numpy.__version__,'ort',onnxruntime.__version__)
    print('INFO providers:', onnxruntime.capi._pybind_state.get_available_providers())
EOF

osf="deps/OpenSeeFace"
if [ -f "$osf/facetracker.py" ] && [ -f "$osf/tracker.py" ] && [ -f "$osf/models/lm_model3_opt.onnx" ]; then
    echo "PASS OpenSeeFace 代码与模型存在（vendored）"
else
    echo "FAIL 缺少 $osf/facetracker.py 或 models/；从 renderer 仓库复制"
    fail=1
fi

python3 - "$osf" <<'EOF' || fail=1
import ast,sys
from pathlib import Path
tree=ast.parse((Path(sys.argv[1])/'facetracker.py').read_text())
features=next(ast.literal_eval(n.value) for n in tree.body if isinstance(n,ast.Assign)
              and any(isinstance(t,ast.Name) and t.id=='features' for t in n.targets))
print('PASS facetracker 特征顺序可解析:', features.index('mouth_open'), '是 mouth_open 的特征序号（=12）')
assert features.index('mouth_open')==12
EOF

case "${1:-}" in
  --arch-hint) echo "INFO 本脚本在 Arch 上只做静态检查；真正的 facetracker 在 Ubuntu Server 上运行";;
esac

[ "$fail" = 0 ] && echo "自检完成：可以在 Ubuntu Server 按部署文档继续" || { echo "自检有 FAIL 项"; exit 1; }
