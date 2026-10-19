#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
EAgent 资产漂移校验器 v3（双层结构适配）—— 只读工具（--manifest 时写 scripts/MANIFEST.json）

双层结构（ADR-0002 前奏，见 scripts/README.md）：
  scripts/frozen/  —— 字节冻结基线：必须与上游二进制内常量逐字节一致（本工具强制校验）
  scripts/working/ —— 工作版本：允许持续改进，与 frozen 的漂移会被**报告但不判失败**

用法:
  python tools/verify_assets.py --all             # 三源全验（_EAgent.e / code.e / EAgent.exe）
  python tools/verify_assets.py --target EAgent.exe
  python tools/verify_assets.py --manifest        # 生成/刷新 scripts/MANIFEST.json（哈希清单）
  python tools/verify_assets.py --manifest-only   # 只生成清单不做三源校验

退出码: 0=通过（frozen 无漂移）；1=frozen 漂移或提取失败
"""
import argparse, hashlib, json, os, subprocess, sys, shutil

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPT_NAMES = ['1_启动调试浏览器.ps1', '2_发送并等待回复.ps1', '3_关闭浏览器.ps1']
TARGETS = ['baseline/upstream/_EAgent.e', 'baseline/upstream/code.e', 'EAgent.exe']

def sha(path):
    return hashlib.sha256(open(path, 'rb').read()).hexdigest()

def canon_path(name):
    """frozen 层为防漂移校验基准；prompt 仍以 baseline 提取件为准"""
    if name == 'prompt_initial.gbk.txt':
        return os.path.join('baseline', 'exact_scripts', 'src', name)
    return os.path.join('scripts', 'frozen', name)

CANON = [(n, canon_path(n)) for n in SCRIPT_NAMES + ['prompt_initial.gbk.txt']]

def verify_target(target: str) -> int:
    label = os.path.basename(target).replace('.', '_')
    tmp = os.path.join(ROOT, 'baseline', '.verify_tmp', label)
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp, exist_ok=True)
    r = subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'extract_assets.py'),
                        '--target', target, '--outdir', tmp, '--label', 'verify'],
                       capture_output=True, text=True, encoding='utf-8', errors='replace',
                       env={**os.environ, 'PYTHONIOENCODING': 'utf-8'}, cwd=ROOT)
    if r.returncode != 0:
        print(f'[FAIL] 提取器返回 {r.returncode}'); print(r.stdout[-2000:]); print(r.stderr[-2000:])
        return 1
    bad = 0
    print(f'--- verify(frozen): {target} ---')
    for name, canon_rel in CANON:
        got = os.path.join(tmp, name)
        want = os.path.join(ROOT, canon_rel)
        if not os.path.exists(got):
            print(f'  [MISS] {name} 未提取到（目标中不存在该资产）'); bad += 1; continue
        if not os.path.exists(want):
            print(f'  [MISS] 基准文件缺失: {canon_rel}'); bad += 1; continue
        a, b = sha(got), sha(want)
        if a == b:
            print(f'  [SAME] {name}  {a[:16]}…')
        else:
            bad += 1
            print(f'  [DRIFT] {name}\n         target={a}\n         frozen={b}')
    shutil.rmtree(tmp, ignore_errors=True)
    return 1 if bad else 0

def report_layers() -> None:
    """frozen vs working 对照（只报告，不判失败）"""
    print('--- layer check: scripts/working vs scripts/frozen ---')
    for name in SCRIPT_NAMES:
        p_fr = os.path.join(ROOT, 'scripts', 'frozen', name)
        p_wk = os.path.join(ROOT, 'scripts', 'working', name)
        if not (os.path.exists(p_fr) and os.path.exists(p_wk)):
            print(f'  [MISS] {name}: 双层不完整'); continue
        h_fr, h_wk = sha(p_fr), sha(p_wk)
        tag = 'SAME' if h_fr == h_wk else 'DRIFTED(允许)'
        print(f'  [{tag}] {name}  frozen={h_fr[:16]}… working={h_wk[:16]}…')

def write_manifest() -> int:
    mf = {'note': 'EAgent 脚本双层清单：frozen=字节冻结基线(参与与上游二进制的防漂移校验)；working=实际运行版本(允许持续改进)。由 tools/verify_assets.py --manifest 生成。',
          'files': {}}
    ok = True
    for name in SCRIPT_NAMES:
        entry = {}
        for layer in ('frozen', 'working'):
            p = os.path.join(ROOT, 'scripts', layer, name)
            if os.path.exists(p):
                entry[layer] = sha(p)
            else:
                entry[layer] = None; ok = False
        mf['files'][name] = entry
    out = os.path.join(ROOT, 'scripts', 'MANIFEST.json')
    with open(out, 'w', encoding='utf-8') as f:
        json.dump(mf, f, ensure_ascii=False, indent=2)
        f.write('\n')
    print(f'manifest -> {out}')
    return 0 if ok else 1

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--target')
    ap.add_argument('--all', action='store_true')
    ap.add_argument('--manifest', action='store_true', help='校验后刷新 scripts/MANIFEST.json')
    ap.add_argument('--manifest-only', action='store_true', help='只生成清单')
    args = ap.parse_args()

    if args.manifest_only:
        return write_manifest()

    rc = 0
    if args.all:
        for t in TARGETS:
            rc |= verify_target(os.path.join(ROOT, t))
    elif args.target:
        rc = verify_target(args.target if os.path.isabs(args.target) else os.path.join(ROOT, args.target))
    else:
        ap.print_help(); return 2

    report_layers()
    if args.manifest:
        rc |= write_manifest()
    print('VERIFY RESULT:', 'PASS' if rc == 0 else 'FAIL')
    return rc

if __name__ == '__main__':
    sys.exit(main())
