# EAgent 阶段1 · 启动冒烟测试（原型验证，无破坏性副作用）
# 验证加固方案的关键假设：
#   1) 随机高位端口启动调试浏览器（不用固定 9222）
#   2) profile 落在 D 盘测试目录（不用 %TEMP%）
#   3) **不带 --remote-allow-origins=*** 时，PowerShell/ClientWebSocket 能否连通 CDP（决定 P0-5 能否直接移除该旗标）
# 结束时会通过 CDP 关闭自己启动的浏览器实例；不影响任何已存在的浏览器会话。
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File tests\smoke_launch.ps1
param(
    [int]$Port = 0,   # 0 = 自动选择空闲端口
    [string]$UserDataDir = "D:\EAgent\baseline\smoke_profile",
    [string]$Url = "about:blank",
    [int]$WaitReadySec = 20
)
$ErrorActionPreference = 'Stop'
function Say([string]$s) { Write-Host $s }

# 1) 找浏览器
$candidates = @(
    "${env:ProgramFiles}\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles}\Microsoft\Edge\Application\msedge.exe",
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
)
$browser = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser) { Say 'FAIL: 未找到 Chrome/Edge'; exit 1 }
Say ("BROWSER: " + $browser)

# 2) 端口：0 -> 随机空闲高位端口（8000-59999）
if ($Port -eq 0) {
    $rnd = New-Object System.Random
    do {
        $Port = $rnd.Next(8000, 60000)
        $busy = $false
        try { $t = New-Object System.Net.Sockets.TcpClient; $t.Connect('127.0.0.1', $Port); $t.Close(); $busy = $true } catch {}
    } while ($busy)
}
Say ("PORT: " + $Port)

# 3) 启动（不带 --remote-allow-origins=*，最小化窗口，避免抢占焦点）
New-Item -ItemType Directory -Force -Path $UserDataDir | Out-Null
$args = @(
    "--remote-debugging-port=$Port",
    "--user-data-dir=$UserDataDir",
    "--no-first-run",
    "--no-default-browser-check",
    $Url
)
$proc = Start-Process -FilePath $browser -ArgumentList $args -WindowStyle Minimized -PassThru
Say ("STARTED: pid=" + $proc.Id)

# 4) 等就绪
$ready = $false
$ver = $null
for ($i = 0; $i -lt ($WaitReadySec * 2); $i++) {
    Start-Sleep -Milliseconds 500
    try { $ver = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/version" -TimeoutSec 2; if ($ver.webSocketDebuggerUrl) { $ready = $true; break } } catch {}
}
if (-not $ready) { Say 'FAIL: /json/version 未就绪'; try { $proc | Stop-Process -Force } catch {}; exit 2 }
Say ("READY: Browser=" + $ver.Browser + "  ws=" + $ver.webSocketDebuggerUrl)

# 5) 关键实验：无 allow-origins 旗标时，ClientWebSocket 连接 + Runtime.evaluate
function WsEval([string]$wsUrl, [string]$expr) {
    $ws = New-Object System.Net.WebSockets.ClientWebSocket
    $ws.ConnectAsync([Uri]$wsUrl, [Threading.CancellationToken]::None).Wait(5000) | Out-Null
    if ($ws.State -ne 'Open') { return @{ ok = $false; why = "WS state=$($ws.State)" } }
    $payload = @{ id = 1; method = 'Runtime.evaluate'; params = @{ expression = $expr; returnByValue = $true } } | ConvertTo-Json -Depth 10 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
    $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$bytes)
    $ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).Wait(3000) | Out-Null
    $buf = New-Object byte[] 65536
    $rseg = New-Object System.ArraySegment[byte] -ArgumentList @(,$buf)
    $task = $ws.ReceiveAsync($rseg, [Threading.CancellationToken]::None)
    if (-not $task.Wait(5000)) { $ws.Dispose(); return @{ ok = $false; why = 'recv timeout' } }
    $text = [Text.Encoding]::UTF8.GetString($buf, 0, $task.Result.Count)
    $ws.Dispose()
    return @{ ok = $true; raw = $text }
}

$pages = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json" -TimeoutSec 5
$page = $pages | Where-Object { $_.type -eq 'page' } | Select-Object -First 1
if ($page) {
    Say ("WS-TEST(page): " + $page.url)
    try {
        $res = WsEval $page.webSocketDebuggerUrl '6*7'
        if ($res.ok -and $res.raw -match '"value"\s*:\s*42') { Say 'PASS: 无 allow-origins 也能 Runtime.evaluate (=42)' }
        elseif ($res.ok) { Say ('PARTIAL: WS 连通但结果异常: ' + $res.raw.Substring(0, [Math]::Min(200, $res.raw.Length))) }
        else { Say ('FAIL: WS 连接/求值失败: ' + $res.why) }
    } catch { Say ('FAIL: WS 异常: ' + $_.Exception.Message) }
} else { Say 'WARN: 没有 page 目标，跳过 WS 实验' }

# 6) 浏览器级 WS（连通性）
try {
    $res = WsEval $ver.webSocketDebuggerUrl '1'
    if ($res.ok) { Say 'PASS: 浏览器级 WS 连通（无 allow-origins）' }
    else { Say ('WARN: 浏览器级 WS: ' + $res.why) }
} catch { Say ('WARN: 浏览器级 WS 异常: ' + $_.Exception.Message) }
# 7) 关闭自身实例
$bws = New-Object System.Net.WebSockets.ClientWebSocket
try {
    $bws.ConnectAsync([Uri]$ver.webSocketDebuggerUrl, [Threading.CancellationToken]::None).Wait(3000) | Out-Null
    $close = @{ id = 99; method = 'Browser.close' } | ConvertTo-Json -Compress
    $b = [Text.Encoding]::UTF8.GetBytes($close)
    $s = New-Object System.ArraySegment[byte] -ArgumentList @(,$b)
    $bws.SendAsync($s, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).Wait(3000) | Out-Null
    $bws.Dispose()
    Say 'CLOSE: Browser.close 已发送'
} catch { Say ('CLOSE: Browser.close 失败，改为 Stop-Process pid=' + $proc.Id); try { $proc | Stop-Process -Force } catch {} }

Start-Sleep -Seconds 2
try { if (-not $proc.HasExited) { $proc | Stop-Process -Force; Say 'CLEANUP: 进程已强制结束' } else { Say 'CLEANUP: 浏览器已正常退出' } } catch {}
Say 'SMOKE DONE'
exit 0
