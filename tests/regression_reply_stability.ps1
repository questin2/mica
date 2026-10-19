# EAgent 20 轮"0 截断"回归（P1-6 验证）
# 原理: 2_发送并等待回复.ps1 的完成判定=文本连续 2.1s 稳定(启发式)。若 AI 仍在生成(思考停顿>2.1s),
#       脚本会提前返回 → 截断。本回归在每轮返回后再等 -SettleSec 秒"偷看"一次:
#       若文本仍在增长(> +20 字符) → 判定该轮截断。
# 前置: 调试浏览器运行中(默认 9222) + 已登录 DeepSeek + 页面停在 chat.deepseek.com。
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File tests\regression_reply_stability.ps1 -Port 9222 -Rounds 20
# 注意: 会真实消耗 N 轮对话; 结果日志写入 baseline\regression_run.log
param(
    [int]$Port = 9222,
    [int]$Rounds = 20,
    [string]$PromptFile = "",
    [int]$SettleSec = 8,
    [int]$ScriptTimeoutSec = 300
)
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if (-not $PromptFile) { $PromptFile = Join-Path $PSScriptRoot 'prompts\long_thinking.txt' }
$sendScript = Join-Path $root 'scripts\working\2_发送并等待回复.ps1'
$peekScript = Join-Path $root 'tools\cdp_peek.ps1'
$logPath = Join-Path $root 'baseline\regression_run.log'

$prompts = Get-Content $PromptFile -Encoding UTF8 | Where-Object { $_.Trim() -ne '' -and -not $_.TrimStart().StartsWith('#') }
if ($prompts.Count -eq 0) { Write-Host 'FAIL: 无可用样本'; exit 1 }
Write-Host ("样本 {0} 条; 轮数 {1}; 结算等待 {2}s" -f $prompts.Count, $Rounds, $SettleSec)

$truncated = 0; $timeout = 0; $ok = 0
$lines = @("=== regression run @ $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss') ===")
for ($i = 1; $i -le $Rounds; $i++) {
    $prompt = $prompts[($i - 1) % $prompts.Count]
    $b64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($prompt))
    $t0 = Get-Date
    $out = & powershell -NoProfile -ExecutionPolicy Bypass -File $sendScript -Port $Port -MessageB64 $b64 2>&1 | Out-String
    $elapsed = [Math]::Round(((Get-Date) - $t0).TotalSeconds, 1)
    if ($out -match 'OK: len=(\d+)') {
        $reported = [int]$Matches[1]
        Start-Sleep -Seconds $SettleSec
        $peek = & powershell -NoProfile -ExecutionPolicy Bypass -File $peekScript -Port $Port 2>&1 | Out-String
        $curLen = -1
        if ($peek -match 'PEEK: assistantMsgs=\d+ len=(\d+)') { $curLen = [int]$Matches[1] }
        $delta = if ($curLen -ge 0) { $curLen - $reported } else { -999 }
        if ($curLen -lt 0) { $verdict = 'PEEK_FAIL' }
        elseif ($delta -gt 20) { $verdict = 'TRUNCATED'; $truncated++ }
        else { $verdict = 'STABLE'; $ok++ }
        $line = "[round {0,2}] script=OK {1,5}s reported={2} peek={3} delta={4} -> {5}" -f $i, $elapsed, $reported, $curLen, $delta, $verdict
    } elseif ($out -match 'TIMEOUT') {
        $timeout++; $verdict = 'TIMEOUT'
        $line = "[round {0,2}] script=TIMEOUT {1,5}s" -f $i, $elapsed
    } else {
        $verdict = 'SCRIPT_FAIL'
        $line = "[round {0,2}] script=FAIL {1,5}s :: {2}" -f $i, $elapsed, ($out.Trim() -replace "`r?`n", ' | ').Substring(0, [Math]::Min(160, $out.Trim().Length))
    }
    Write-Host $line
    $lines += $line
}
$summary = "SUMMARY: 稳定 {0}/{1}  截断 {2}  超时 {3}  失败 {4}" -f $ok, $Rounds, $truncated, $timeout, ($Rounds - $ok - $truncated - $timeout)
Write-Host $summary
$lines += $summary
$lines | Out-File -FilePath $logPath -Encoding UTF8 -Append
Write-Host ("日志: " + $logPath)
if ($truncated -eq 0) { exit 0 } else { exit 1 }
