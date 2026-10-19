# EAgent 阶段1 · 静态测试套件（无副作用）
# T1 脚本语法   T2 仓库内字节一致性(scripts/==baseline/exact_scripts/src)   T3 selectors.json 有效性
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$fail = 0

Write-Host '=== T0 .ps1 文件 BOM 检查 (PS5.1 需要 UTF-8 BOM) ==='
$psFiles = @()
$psFiles += Get-ChildItem (Join-Path $root 'scripts\frozen')  -Filter *.ps1
$psFiles += Get-ChildItem (Join-Path $root 'scripts\working') -Filter *.ps1
$psFiles += Get-ChildItem (Join-Path $root 'tests')   -Filter *.ps1
$psFiles += Get-ChildItem (Join-Path $root 'tools')   -Filter *.ps1
$t0 = 0
foreach ($f in $psFiles) {
    $b = [IO.File]::ReadAllBytes($f.FullName)
    if (-not ($b.Length -ge 3 -and $b[0] -eq 0xEF -and $b[1] -eq 0xBB -and $b[2] -eq 0xBF)) {
        Write-Host ("NO-BOM {0}" -f $f.FullName)
        $t0++
    }
}
if ($t0 -eq 0) { Write-Host ("T0 通过: {0} 个 .ps1 均含 UTF-8 BOM" -f $psFiles.Count) } else { $fail += $t0 }

Write-Host ''
Write-Host '=== T1 脚本语法 (AST 解析) ==='
& (Join-Path $PSScriptRoot 'parse_check.ps1')
if ($LASTEXITCODE -ne 0) { $fail++ }

Write-Host ''
Write-Host '=== T2 scripts/frozen 与 baseline/exact_scripts/src 字节一致性（必须一致） ==='
$srcDir = Join-Path $root 'baseline\exact_scripts\src'
$t2 = 0
foreach ($f in Get-ChildItem (Join-Path $root 'scripts\frozen') -Filter *.ps1) {
    $base = Join-Path $srcDir $f.Name
    if (-not (Test-Path $base)) { Write-Host ("FAIL  {0}: 基准缺失" -f $f.Name); $t2++; continue }
    $h1 = (Get-FileHash $f.FullName -Algorithm SHA256).Hash
    $h2 = (Get-FileHash $base -Algorithm SHA256).Hash
    if ($h1 -eq $h2) { Write-Host ("SAME(frozen)  {0}" -f $f.Name) }
    else { Write-Host ("DRIFT {0}`n   frozen={1}`n   baseline={2}" -f $f.Name, $h1, $h2); $t2++ }
}
if ($t2 -eq 0) { Write-Host 'T2a 通过: frozen/ 与提取基准逐字节一致' } else { $fail += $t2 }

Write-Host ''
Write-Host '=== T2b scripts/working vs scripts/frozen（允许漂移，仅报告） ==='
foreach ($f in Get-ChildItem (Join-Path $root 'scripts\working') -Filter *.ps1) {
    $fr = Join-Path (Join-Path $root 'scripts\frozen') $f.Name
    if (-not (Test-Path $fr)) { Write-Host ("WARN  working/{0}: frozen 无对应文件" -f $f.Name); continue }
    $h1 = (Get-FileHash $f.FullName -Algorithm SHA256).Hash
    $h2 = (Get-FileHash $fr -Algorithm SHA256).Hash
    if ($h1 -eq $h2) { Write-Host ("SAME(working==frozen)  {0}" -f $f.Name) }
    else { Write-Host ("DRIFTED(working!=frozen, 允许)  {0}`n   working={1}`n   frozen ={2}" -f $f.Name, $h1, $h2) }
}

Write-Host ''
Write-Host '=== T3 config/selectors.json 有效性 ==='
$selPath = Join-Path $root 'config\selectors.json'
if (-not (Test-Path $selPath)) { Write-Host 'FAIL: 缺少 config/selectors.json'; $fail++ }
else {
    try {
        $j = Get-Content $selPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $bad = 0
        if ([int]$j.version -lt 2) { Write-Host ("FAIL: version={0} (期望>=2)" -f $j.version); $bad++ }
        if (-not $j.completion -or -not $j.completion.timeoutSec) { Write-Host 'FAIL: 缺少 completion.timeoutSec'; $bad++ }
        else { Write-Host ("OK    completion: timeout={0}s poll={1}ms primary={2}" -f $j.completion.timeoutSec, $j.completion.pollMs, $j.completion.primary) }
        $need = @('assistant', 'user', 'input', 'sendBtn', 'fileInput')
        foreach ($k in $need) {
            $g = $j.groups.$k
            if (-not $g -or -not $g.candidates -or $g.candidates.Count -eq 0) { Write-Host ("FAIL: groups.{0} 候选为空" -f $k); $bad++; continue }
            $emptySel = @($g.candidates | Where-Object { -not $_.selector }).Count
            if ($emptySel -gt 0) { Write-Host ("FAIL: groups.{0} 有 {1} 个候选缺少 selector" -f $k, $emptySel); $bad++; continue }
            Write-Host ("OK    groups.{0}: {1} 个候选 (severity={2})" -f $k, $g.candidates.Count, $g.severity)
        }
        if ($bad -gt 0) { $fail += $bad } else { Write-Host 'T3 通过 (schema v2)' }
    } catch { Write-Host ("FAIL: JSON 解析错误: {0}" -f $_.Exception.Message); $fail++ }
}

Write-Host ''
Write-Host '=== T4 prompt 资产存在性 ==='
$p1 = Join-Path $root 'baseline\exact_scripts\src\prompt_initial.gbk.txt'
$p2 = Join-Path $root 'config\prompt.initial.md'
if ((Test-Path $p1) -and (Test-Path $p2)) { Write-Host 'OK: prompt 字节级 + 可读版均存在' }
else { Write-Host 'FAIL: prompt 资产缺失'; $fail++ }

Write-Host ''
Write-Host '=== T5 scripts/MANIFEST.json 哈希清单校验 ==='
$mf = Join-Path $root 'scripts\MANIFEST.json'
if (-not (Test-Path $mf)) { Write-Host 'WARN: 未生成 scripts/MANIFEST.json（运行: python tools\verify_assets.py --manifest）' }
else {
    $m = Get-Content $mf -Raw -Encoding UTF8 | ConvertFrom-Json
    $t5 = 0
    foreach ($prop in $m.files.PSObject.Properties) {
        foreach ($layer in @('frozen', 'working')) {
            $want = $prop.Value.$layer
            $disk = Join-Path $root ("scripts\{0}\{1}" -f $layer, $prop.Name)
            if (-not (Test-Path $disk)) { Write-Host ("FAIL  manifest 记录的文件缺失: {0}/{1}" -f $layer, $prop.Name); $t5++; continue }
            $got = (Get-FileHash $disk -Algorithm SHA256).Hash.ToLower()
            if ($got -eq ([string]$want).ToLower()) { Write-Host ("OK    {0}/{1}" -f $layer, $prop.Name) }
            else { Write-Host ("STALE {0}/{1}`n   manifest={2}`n   disk    ={3}" -f $layer, $prop.Name, $want, $got); $t5++ }
        }
    }
    if ($t5 -eq 0) { Write-Host 'T5 通过: 清单与磁盘一致' } else { Write-Host 'T5: 清单过期，请运行 verify_assets.py --manifest 刷新'; $fail += $t5 }
}

Write-Host ''
if ($fail -gt 0) { Write-Host ("测试未全部通过: {0} 项失败" -f $fail); exit 1 }
Write-Host '全部静态测试通过 (T0-T5)'
exit 0
