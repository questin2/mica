# EAgent 通道自检（只读，无副作用）
# 在已运行的调试浏览器上，统计 config/selectors.json 每组候选的命中数，
# 并检测发送按钮状态识别（stop/arrow）。不注入、不发送、不点击。
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File tools\channel_selfcheck.ps1 -Port 9222
param([int]$Port = 9222)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$selPath = Join-Path $root 'config\selectors.json'
if (-not (Test-Path $selPath)) { Write-Output ('ERROR: missing ' + $selPath); exit 1 }
$sel = Get-Content $selPath -Raw -Encoding UTF8 | ConvertFrom-Json

# 1) 找 DeepSeek 页面
try { $pages = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json" -TimeoutSec 5 }
catch { Write-Output ("ERROR: debug port $Port not reachable"); exit 2 }
$target = $pages | Where-Object { $_.url -match 'chat\.deepseek\.com' -and $_.type -eq 'page' } | Select-Object -First 1
if (-not $target) { Write-Output 'ERROR: no chat.deepseek.com page found'; exit 2 }
Write-Output ("PAGE: " + $target.url)

# 2) 建 WS
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

# 3) 组装检查清单并求值（纯查询）
$checks = @()
foreach ($g in $sel.groups.PSObject.Properties) {
    foreach ($c in $g.Value.candidates) {
        $checks += [pscustomobject]@{ group = $g.Name; selector = $c.selector; kind = $c.kind }
    }
}
$checksJson = $checks | ConvertTo-Json -Compress
$js = @"
(function(){
    var C = $checksJson;
    var out = [];
    C.forEach(function(c){
        var n = 0;
        try { n = document.querySelectorAll(c.selector).length; } catch(e) { n = -1; }
        var vis = 0;
        try {
            document.querySelectorAll(c.selector).forEach(function(el){ if (el.offsetParent !== null) vis++; });
        } catch(e) {}
        out.push({ group: c.group, selector: c.selector, kind: c.kind, total: n, visible: vis });
    });
    // 发送按钮状态（用第一组命中）
    var btn = document.querySelector('div[role="button"].ds-button--primary.ds-button--circle');
    var state = { found: !!btn, isStop: null, isArrow: null };
    if (btn) {
        var p = btn.querySelector('svg path');
        var d = p ? (p.getAttribute('d') || '') : '';
        state.isStop  = d.indexOf('M2 4.88C2 3.68009') === 0;
        state.isArrow = d.indexOf('M8.3125 0.980206') === 0;
    }
    return JSON.stringify({ checks: out, sendState: state });
})()
"@
$r = CDPCall 'Runtime.evaluate' @{ expression = $js; returnByValue = $true } 8000
$ws.Dispose()
if (-not $r -or -not $r.result) { Write-Output 'ERROR: evaluate failed'; exit 4 }

$data = $r.result.value | ConvertFrom-Json
Write-Output ''
Write-Output '===== 选择器命中 ====='
foreach ($c in $data.checks) {
    Write-Output ("[{0}] {1,-5} total={2,-3} visible={3,-3}  {4}" -f $c.group, $c.kind, $c.total, $c.visible, $c.selector)
}
Write-Output ''
$s = $data.sendState
if ($s.found) { Write-Output ("发送按钮: found=true isStop={0} isArrow={1}" -f $s.isStop, $s.isArrow) }
else { Write-Output '发送按钮: 未找到（页面可能未加载聊天界面）' }
Write-Output ''
Write-Output '注: 请将 verified=true 手工回填到 config/selectors.json（本脚本只读，不修改文件）。'
exit 0
