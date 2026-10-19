# EAgent CDP 偷看器（只读，无副作用）
# 读取当前 DeepSeek 页面最后一条助手消息的文本长度与尾部。用于：
#   1) 回归测试的"偷看"步骤（判断回复是否还在增长 → 截断检出）
#   2) 日常排查（不注入/不发送/不点击）
# 本脚本是 config/selectors.json 的第一个消费者（选择器来自外置配置）。
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File tools\cdp_peek.ps1 -Port 9222 [-TailChars 60]
param(
    [int]$Port = 9222,
    [int]$TailChars = 60
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$selPath = Join-Path $root 'config\selectors.json'
$sel = Get-Content $selPath -Raw -Encoding UTF8 | ConvertFrom-Json

function Pick-Json([string]$group) {
    $arr = @($sel.groups.$group.candidates | ForEach-Object { $_.selector })
    return ($arr | ConvertTo-Json -Compress)
}

try { $pages = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json" -TimeoutSec 5 }
catch { Write-Output "ERROR: debug port $Port not reachable"; exit 2 }
$target = $pages | Where-Object { $_.url -match 'chat\.deepseek\.com' -and $_.type -eq 'page' } | Select-Object -First 1
if (-not $target) { Write-Output 'ERROR: no chat.deepseek.com page'; exit 2 }

$ws = New-Object System.Net.WebSockets.ClientWebSocket
$ws.ConnectAsync([Uri]$target.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait(5000) | Out-Null
if ($ws.State -ne 'Open') { Write-Output 'ERROR: WS not open'; exit 3 }

$script:cdpId = 0
function CDPRecv([int]$id, [int]$timeoutMs) {
    $buf = New-Object byte[] 1048576
    $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$buf)
    $deadline = (Get-Date).AddMilliseconds($timeoutMs)
    while ((Get-Date) -lt $deadline) {
        $msLeft = [int]($deadline - (Get-Date)).TotalMilliseconds
        if ($msLeft -lt 1) { return $null }
        $task = $ws.ReceiveAsync($seg, [Threading.CancellationToken]::None)
        if (-not $task.Wait($msLeft)) { return $null }
        $rr = $task.Result
        if ($rr -and $rr.Count -gt 0) {
            $text = [Text.Encoding]::UTF8.GetString($buf, 0, $rr.Count)
            foreach ($line in ($text -split "`n")) {
                if ($line -match ('"id"\s*:\s*' + $id + '\b')) {
                    try { return ($line | ConvertFrom-Json) } catch {}
                }
            }
        }
    }
    return $null
}
function CDPCall([string]$method, $params, [int]$timeoutMs = 8000) {
    $script:cdpId++; $id = $script:cdpId
    $payload = @{ id = $id; method = $method; params = $params } | ConvertTo-Json -Depth 20 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
    $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$bytes)
    $ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).Wait(3000) | Out-Null
    $resp = CDPRecv $id $timeoutMs
    if ($resp -and $resp.result) { return $resp.result }
    return $null
}

$assistantJson = Pick-Json 'assistant'
$js = @"
(function(){
    var list = $assistantJson;
    var els = [];
    for (var i = 0; i < list.length; i++) {
        var found = document.querySelectorAll(list[i]);
        for (var j = 0; j < found.length; j++) { if (els.indexOf(found[j]) < 0) els.push(found[j]); }
    }
    if (els.length === 0) return JSON.stringify({ count: 0, len: 0, tail: '' });
    var t = (els[els.length - 1].textContent || '').trim();
    return JSON.stringify({ count: els.length, len: t.length, tail: t.slice(-$TailChars) });
})()
"@
$r = CDPCall 'Runtime.evaluate' @{ expression = $js; returnByValue = $true } 8000
$ws.Dispose()
if (-not $r -or -not $r.result) { Write-Output 'ERROR: evaluate failed'; exit 4 }
$d = $r.result.value | ConvertFrom-Json
Write-Output ("PEEK: assistantMsgs={0} len={1}" -f $d.count, $d.len)
if ($d.tail) { Write-Output ("TAIL: " + ($d.tail -replace "`r?`n", ' ')) }
exit 0
