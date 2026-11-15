#!/usr/bin/env python3
"""修补 vendored OpenSeeFace 的 NumPy 兼容性：np.float/np.int 等 1.20 弃用别名。
幂等：已修补的文件会跳过。用法：python3 fix_numpy_aliases.py [OpenSeeFace目录]
"""
import re
import sys
from pathlib import Path

REPLACEMENTS = [
    # 精确词边界替换，避免误伤 np.float32 / np.float64 等
    (r'\bnp\.float\b(?![_\w])', 'float'),
    (r'\bnp\.int\b(?![_\w])', 'int'),
    (r'\bnp\.bool\b(?![_\w])', 'bool'),
    (r'\bnp\.object\b(?![_\w])', 'object'),
    (r'\bnp\.complex\b(?![_\w])', 'complex'),
    (r'\bnp\.str\b(?![_\w])', 'str'),
]

def patch_file(path: Path) -> int:
    text = path.read_text(encoding='utf-8', errors='replace')
    original = text
    count = 0
    for pattern, repl in REPLACEMENTS:
        text, n = re.subn(pattern, repl, text)
        count += n
    if count and text != original:
        path.write_text(text, encoding='utf-8')
    return count

def main():
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path('.')
    if not root.is_dir():
        raise SystemExit(f'目录不存在: {root}')
    total = 0
    files = 0
    for py in sorted(root.rglob('*.py')):
        n = patch_file(py)
        if n:
            print(f'{py.relative_to(root)}: 替换 {n} 处')
            total += n
            files += 1
    print(f'\n完成：{files} 个文件共替换 {total} 处' if total else '\n没有需要修补的地方（已修补过）')

if __name__ == '__main__':
    main()
