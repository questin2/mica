# Authenticode 签名（OV/EV 证书）
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts\release\sign_windows.ps1 `
#     -PfxPath D:\secrets\ov.pfx -Password <pfx密码> `
#     -Files "app\src-tauri\target\release\bundle\nsis\EAgent_0.1.1_x64-setup.exe", `
#            "app\src-tauri\target\release\EAgent.exe"
#
# 说明：
#   - 这是与 updater minisign **独立**的第二条签名链；
#   - 时间戳服务器默认 DigiCert（需联网）；签名后不可再修改产物；
#   - 证书必须是 Code Signing 用途；OV 新证书仍可能触发 SmartScreen 黄屏，EV 才有即时信誉。
param(
  [Parameter(Mandatory = $true)][string]$PfxPath,
  [Parameter(Mandatory = $true)][string]$Password,
  [Parameter(Mandatory = $true)][string[]]$Files,
  [string]$TimestampUrl = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"
$signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin" -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
  Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
  Sort-Object FullName -Descending |
  Select-Object -First 1
if (-not $signtool) { throw "找不到 signtool.exe（安装 Windows SDK）" }
Write-Output "signtool: $($signtool.FullName)"

foreach ($f in $Files) {
  if (-not (Test-Path $f)) { throw "文件不存在：$f" }
  Write-Output "> signing $f"
  & $signtool.FullName sign /f $PfxPath /p $Password /fd SHA256 /td SHA256 /tr $TimestampUrl $f
  if ($LASTEXITCODE -ne 0) { throw "签名失败：$f" }
  $sig = Get-AuthenticodeSignature $f
  Write-Output "  status: $($sig.Status)"
  if ($sig.Status -ne "Valid") {
    Write-Output "  注意：状态不是 Valid（自签名/不受信任根会显示 UnknownError）"
  }
}
