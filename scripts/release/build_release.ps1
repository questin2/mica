# EAgent 发布构建（NSIS + updater minisign）
#
# 用法：
#   powershell -ExecutionPolicy Bypass -File scripts\release\build_release.ps1
#   powershell ... -DebugBuild
#
# 前置：
#   - 已生成 updater 密钥：npx tauri signer generate -w <key> -p <password> --ci
#   - 私钥与密码文件放在仓库外（默认 %APPDATA%\EAgent\secrets，可用 -SigningKey/-PasswordFile 覆盖）
#
# 产物：
#   app/src-tauri/target/<profile>/bundle/nsis/EAgent_<ver>_x64-setup.exe
#   ...exe.sig（updater 签名）
#   latest.json（updater 清单模板；url 需改成实际发布地址）
param(
  [string]$SigningKey = "$env:APPDATA\EAgent\secrets\eagent-updater.key",
  [string]$PasswordFile = "$env:APPDATA\EAgent\secrets\updater_password.txt",
  [string]$Config = "src-tauri/tauri.release.conf.json",
  [string]$ReleaseBaseUrl = "https://github.com/YaoGaoChaoLe/EAgent/releases/download",
  [switch]$DebugBuild
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location (Join-Path $repo "app")

if (-not (Test-Path $SigningKey)) { throw "找不到签名私钥：$SigningKey" }
if (-not (Test-Path $PasswordFile)) { throw "找不到密码文件：$PasswordFile" }

$env:TAURI_SIGNING_PRIVATE_KEY = (Resolve-Path $SigningKey).Path
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = (Get-Content $PasswordFile -Raw).Trim()

$args = @("tauri", "build", "--config", $Config)
if ($DebugBuild) { $args += "--debug" }
Write-Output "> npx $($args -join ' ')"
& npx @args
if ($LASTEXITCODE -ne 0) { throw "tauri build 失败（exit $LASTEXITCODE）" }

$profile = if ($DebugBuild) { "debug" } else { "release" }
$nsis = Join-Path $repo "app\src-tauri\target\$profile\bundle\nsis"
$conf = Get-Content (Join-Path $repo "app\src-tauri\tauri.conf.json") -Raw | ConvertFrom-Json
$ver = $conf.version
$installer = Join-Path $nsis "EAgent_${ver}_x64-setup.exe"
$sigFile = "$installer.sig"
if (-not (Test-Path $installer)) { throw "找不到安装包：$installer" }
if (-not (Test-Path $sigFile)) { throw "找不到 updater 签名：$sigFile" }

$sig = (Get-Content $sigFile -Raw).Trim()
$url = "$ReleaseBaseUrl/v$ver/EAgent_${ver}_x64-setup.exe"
$latest = [ordered]@{
  version = $ver
  notes = "EAgent $ver"
  pub_date = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  platforms = @{
    "windows-x86_64" = @{
      signature = $sig
      url = $url
    }
  }
}
$latestPath = Join-Path $nsis "latest.json"
$latest | ConvertTo-Json -Depth 6 | Set-Content -Path $latestPath -Encoding UTF8

Write-Output ""
Write-Output "构建完成："
Write-Output "  安装包   : $installer"
Write-Output "  updater 签名: $sigFile"
Write-Output "  latest.json: $latestPath"
Write-Output ""
Write-Output "发布前还需："
Write-Output "  1. Authenticode 签名（scripts\release\sign_windows.ps1）"
Write-Output "  2. 把 latest.json 里的 url 改成实际上传地址"
Write-Output "  3. 上传安装包 + .sig + latest.json 到更新源"
Write-Output "  4. 在干净用户/VM 跑 R01-R03 与 R05"
