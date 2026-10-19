# EAgent 阶段1 · 脚本语法检查（静态，无副作用）
# 解析 scripts/*.ps1 的 AST，不执行任何脚本体。
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$files = @()
$files += Get-ChildItem (Join-Path $root 'scripts\frozen')  -Filter *.ps1
$files += Get-ChildItem (Join-Path $root 'scripts\working') -Filter *.ps1
$files = $files | Sort-Object FullName
if ($files.Count -eq 0) { Write-Host 'FAIL: scripts/frozen|working 下没有 .ps1'; exit 1 }

$fail = 0
foreach ($f in $files) {
    $text   = [IO.File]::ReadAllText($f.FullName)   # BOM 自动识别 -> UTF-8
    $tokens = $null
    $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseInput($text, [ref]$tokens, [ref]$errors)
    if ($errors.Count -eq 0) {
        Write-Host ("PASS  {0}/{1}  ({2} chars)" -f $f.Directory.Name, $f.Name, $text.Length)
    } else {
        $fail++
        Write-Host ("FAIL  {0}/{1}" -f $f.Directory.Name, $f.Name)
        foreach ($e in $errors) {
            Write-Host ("      line {0}: {1}" -f $e.Extent.StartLineNumber, $e.Message)
        }
    }
}
if ($fail -gt 0) { Write-Host ("语法检查不通过: {0}/{1}" -f $fail, $files.Count); exit 1 }
Write-Host ("语法检查全部通过 ({0} 个脚本)" -f $files.Count)
exit 0
