param(
  [string]$ProcName = "eagent-app",
  [string]$TitleHint = "EAgent",
  [string]$Out = "D:\EAgent\baseline\ui_cds_run1.png"
)

# 抓取目标进程主窗口的截图（EnumWindows 按进程 + 面积选择，规避 MainWindowTitle 启发式不稳）
# PrintWindow 离屏渲染；仅抓目标窗口矩形，不采集屏幕其他区域（隐私）。
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File tools\capture_window.ps1 -ProcName eagent-app -Out D:\path\shot.png

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WinCap2 {
  public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr hWnd, StringBuilder sb, int max);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
}
"@

$procs = Get-Process -Name $ProcName -ErrorAction SilentlyContinue
if (-not $procs) { Write-Error "process not found: $ProcName"; exit 1 }
$pids = @($procs | ForEach-Object { $_.Id })

$script:cands = @()
$cb = [WinCap2+EnumProc] {
  param($h, $l)
  $p = 0
  [WinCap2]::GetWindowThreadProcessId($h, [ref]$p) | Out-Null
  if ($pids -contains [int]$p) {
    if ([WinCap2]::IsWindowVisible($h)) {
      $sb = New-Object System.Text.StringBuilder 256
      [WinCap2]::GetWindowText($h, $sb, 256) | Out-Null
      $t = $sb.ToString()
      $iconic = [WinCap2]::IsIconic($h)
      $area = 0
      if (-not $iconic) {
        $r = New-Object WinCap2+RECT
        [WinCap2]::GetWindowRect($h, [ref]$r) | Out-Null
        $area = ($r.Right - $r.Left) * ($r.Bottom - $r.Top)
      }
      if ($t -like "*$TitleHint*") { $area = $area + 1000000000 }  # 标题吻合优先
      $script:cands += @{ H = $h; Title = $t; Iconic = $iconic; Area = $area }
    }
  }
  return $true
}
[WinCap2]::EnumWindows($cb, [IntPtr]::Zero) | Out-Null

$best = $script:cands | Sort-Object { $_.Area } -Descending | Select-Object -First 1
if (-not $best) { Write-Error "no visible top-level window for $ProcName"; exit 1 }

# 最小化态：临时还原 → 截图 → 放回最小化（不改变用户桌面状态）
$restore = $false
if ($best.Iconic) {
  [WinCap2]::ShowWindow($best.H, 9) | Out-Null   # SW_RESTORE
  Start-Sleep -Milliseconds 900
  $restore = $true
}
$r = New-Object WinCap2+RECT
[WinCap2]::GetWindowRect($best.H, [ref]$r) | Out-Null
$w = $r.Right - $r.Left
$hgt = $r.Bottom - $r.Top
if ($w -lt 200 -or $hgt -lt 200) {
  if ($restore) { [WinCap2]::ShowWindow($best.H, 6) | Out-Null }
  Write-Error "best window too small ($w x $hgt, title '$($best.Title)') - app may be in the known no-window failure state"
  exit 1
}

$bmp = New-Object System.Drawing.Bitmap($w, $hgt)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
[WinCap2]::PrintWindow($best.H, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc)
$g.Dispose()
$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
if ($restore) { [WinCap2]::ShowWindow($best.H, 6) | Out-Null }  # SW_MINIMIZE 放回
Write-Output "saved: $Out ($w x $hgt) title='$($best.Title)' restored=$restore"
