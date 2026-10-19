#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
EAgent 资产提取器 v2（阶段1 / T1）—— 只读工具

背景（v1 诊断结论）：易语言工程/产物中的字符串常量按记录存放：
    [u32 长度][UTF-8 BOM][文本]（ps1 脚本=UTF-8+BOM; 提示词=GBK 无 BOM）
且同一脚本可能存在多个副本（主版本 + "备份_ps_N" 版本）。

v2 策略：不解析记录头，直接定位所有 "BOM+param(" 副本起点，按类型分类，
以"窗口内最后一个 }" 收尾（窗口边界=下一个副本起点），逐字节导出并校验：
  - UTF-8 严格解码（脚本） / GB18030 解码（提示词）
  - 尾部模式断言
  - 副本间差异对照表

用法:
  python tools/extract_assets.py --target baseline/upstream/_EAgent.e --outdir baseline/exact_scripts --label src_head --report baseline/extract_report.json
"""
import argparse, difflib, hashlib, json, os, re, sys

BOM = b'\xef\xbb\xbf'

FAMILY_TOKENS = [
    ('1_启动调试浏览器', b'[string]$ChromePath'),
    ('2_发送并等待回复', b'[string]$Message'),
    ('3_关闭浏览器',     b'Browser.close'),
]

def classify(tail: bytes) -> str:
    for name, tok in FAMILY_TOKENS:
        if tok in tail:
            return name
    return '未知'

def find_script_copies(data: bytes):
    """定位所有 BOM+param( 的副本，返回 [(start_incl_bom, family, window_end)]"""
    starts = [m.start() for m in re.finditer(re.escape(BOM + b'param('), data)]
    found = []
    for s in starts:
        window = data[s:s + 800]
        fam = classify(window)
        found.append({'start': s, 'family': fam})
    for i, f in enumerate(found):
        f['window_end'] = found[i + 1]['start'] if i + 1 < len(found) else len(data)
    return found

TAIL_CHECKS = {
    '1_启动调试浏览器': re.compile(rb'exit 1\r?\n\s*\}\r?\n?$'),
    '2_发送并等待回复': re.compile(rb'exit 2\r?\n\s*\}\r?\n?$'),
    '3_关闭浏览器':     re.compile(rb'port \$Port"\r?\n\s*\}\r?\n?$'),
}

# 唯一尾部搜索锚（避免体内 "exit 1" 误匹配）: 匹配到该组合的 } 为止
TAIL_SEARCH = {
    '1_启动调试浏览器': re.compile(rb'debug port not ready"\r?\n[ \t]*exit 1\r?\n[ \t]*\}'),
    '2_发送并等待回复': re.compile(rb'TIMEOUT"\r?\n[ \t]*exit 2\r?\n[ \t]*\}'),
    '3_关闭浏览器':     re.compile(rb'not running on port \$Port"\r?\n[ \t]*\}'),
}

def extract_scripts(data: bytes, outdir: str, report: dict):
    os.makedirs(outdir, exist_ok=True)
    copies = find_script_copies(data)
    by_family = {}
    for c in copies:
        s, we, fam = c['start'], c['window_end'], c['family']
        pat = TAIL_SEARCH.get(fam)
        e = -1
        rec_multi = 0
        if pat:
            ms = list(pat.finditer(data, s, min(we, s + 400_000)))
            if ms:
                e = ms[-1].end() - 1   # 取窗口内最后一个（脚本可能有多个分支段）
                rec_multi = len(ms)
        if e < 0:
            e = data.rfind(b'}', s, we)
            report['notes'].append(f'copy {fam}@{s:#x}: tail-search failed, fallback rfind')
        if e < 0:
            report['notes'].append(f'no closing brace for copy at {s:#x}')
            continue
        raw = data[s:e + 1]          # 逐字节，含 BOM
        text = raw[len(BOM):] if raw.startswith(BOM) else raw
        rec = {
            'family': fam, 'offset': s, 'size': len(raw),
            'sha256': hashlib.sha256(raw).hexdigest(),
            'tail_ok': bool(TAIL_CHECKS.get(fam) and TAIL_CHECKS[fam].search(raw)),
            'tail_match_count': rec_multi,
            'bounds_hex_after': data[e + 1:e + 9].hex(),
        }
        try:
            text.decode('utf-8-sig')
            rec['encoding'] = 'utf-8'
        except UnicodeDecodeError as ex:
            rec['encoding'] = 'not-utf8'
            rec['notes'] = str(ex)
        by_family.setdefault(fam, []).append((rec, raw))

    for fam, items in sorted(by_family.items()):
        items = sorted(items, key=lambda x: x[0]['offset'])
        for idx, (rec, raw) in enumerate(items):
            fname = fam + ('' if idx == 0 else f'_副本{idx}') + '.ps1'
            path = os.path.join(outdir, fname)
            with open(path, 'wb') as f:
                f.write(raw)
            rec['written'] = path
            print(f"[copy] {fname:32s} off={rec['offset']:#x} size={rec['size']} enc={rec['encoding']} tail_ok={rec['tail_ok']} after={rec['bounds_hex_after']}")
        # 副本差异
        if len(items) > 1:
            base = items[0][1].decode('utf-8-sig', 'replace')
            for rec, raw in items[1:]:
                other = raw.decode('utf-8-sig', 'replace')
                diff = list(difflib.unified_diff(base.splitlines(), other.splitlines(),
                                                 'main', f"copy@{rec['offset']:#x}", lineterm=''))
                report.setdefault('diffs', {})[f"{fam}@{rec['offset']:#x}"] = diff[:400]
                print(f"[diff] {fam} main vs copy@{rec['offset']:#x}: {len(diff)} diff lines")
    report['script_copies'] = [
        {k: v for k, v in rec.items()} for fam, items in by_family.items() for rec, _ in items
    ]

def extract_prompt(data: bytes, outdir: str, report: dict):
    os.makedirs(outdir, exist_ok=True)
    start_pat = '- 从现在开始'.encode('gb18030')
    end_anchor = '向用户交付最终成品。'.encode('gb18030')
    copies, pos = [], 0
    while True:
        s = data.find(start_pat, pos)
        if s < 0:
            break
        ea = data.find(end_anchor, s, s + 4_000_000)
        if ea >= 0:
            copies.append((s, ea + len(end_anchor)))
        pos = s + 1
    report['prompt_copies'] = len(copies)
    for idx, (s, e) in enumerate(copies):
        raw = data[s:e]
        try:
            text = raw.decode('gb18030')
        except UnicodeDecodeError as ex:
            report['notes'].append(f'prompt copy {idx} decode fail: {ex}')
            continue
        rec = {'name': 'prompt_initial', 'copy': idx, 'offset': s, 'size': len(raw),
               'sha256': hashlib.sha256(raw).hexdigest(),
               'bom_before': data[max(0, s - 3):s] == BOM,
               'head': text[:40], 'tail': text[-30:]}
        if idx == 0:
            path = os.path.join(outdir, 'prompt_initial.gbk.txt')
            open(path, 'wb').write(raw)
            open(os.path.join(outdir, 'prompt_initial.utf8.txt'), 'w', encoding='utf-8').write(text)
            rec['written'] = path
        else:
            path = os.path.join(outdir, f'prompt_initial_副本{idx}.gbk.txt')
            open(path, 'wb').write(raw)
            rec['written'] = path
        report.setdefault('prompt', []).append(rec)
        print(f"[ok] prompt copy{idx} off={s:#x} size={len(raw)} bom_before={rec['bom_before']} -> {path}")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--target', required=True)
    ap.add_argument('--outdir', required=True)
    ap.add_argument('--label', default='')
    ap.add_argument('--report', default=None)
    args = ap.parse_args()

    data = open(args.target, 'rb').read()
    report = {'target': os.path.abspath(args.target), 'label': args.label,
              'size': len(data), 'sha256': hashlib.sha256(data).hexdigest(), 'notes': []}
    extract_scripts(data, args.outdir, report)
    extract_prompt(data, args.outdir, report)

    if args.report:
        hist = []
        if os.path.exists(args.report):
            try:
                hist = json.load(open(args.report, encoding='utf-8'))
            except Exception:
                hist = []
        hist.append(report)
        json.dump(hist, open(args.report, 'w', encoding='utf-8'), ensure_ascii=False, indent=2)
        print(f'report -> {args.report}')

if __name__ == '__main__':
    main()
