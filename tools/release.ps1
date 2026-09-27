<#
.SYNOPSIS
  發新版：改 Cargo.toml 版號 → 跑測試 → commit → 打 tag → 推上 GitHub。
  推上 tag 後由 .github/workflows/release.yml 自動 build 並開 release（附 version.dll + blake3.json）。

.EXAMPLE
  .\tools\release.ps1 0.14.1
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Version
)

$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

function Invoke-Native {
    param([string]$Exe, [string[]]$Arguments)
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Exe $($Arguments -join ' ') 失敗（exit $LASTEXITCODE）" }
}

if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "版號要是 X.Y.Z 格式，收到 '$Version'" }
$tag = "v$Version"

# —— 前置檢查 ——
$branch = (git branch --show-current).Trim()
if ($branch -ne 'main') { throw "只從 main 發版（目前在 $branch）" }
if (git status --porcelain --untracked-files=no) { throw "有未 commit 的改動，先處理掉" }
if (git tag --list $tag) { throw "tag $tag 已存在" }

Invoke-Native git @('fetch', '--quiet', 'origin')
git merge-base --is-ancestor origin/main HEAD
if ($LASTEXITCODE -ne 0) { throw "本機 main 落後或分岔於 origin/main，先 pull" }

# —— 改版號（只改 [package] 那行；讀寫都用無 BOM 的 UTF-8，Cargo.toml 有中文註解）——
$utf8 = New-Object System.Text.UTF8Encoding($false)
$cargoPath = Join-Path (Get-Location) 'Cargo.toml'
$cargo = [IO.File]::ReadAllText($cargoPath, $utf8)
$pattern = '(?m)^version = "(\d+)\.(\d+)\.(\d+)"'
$m = [regex]::Match($cargo, $pattern)
if (-not $m.Success) { throw "Cargo.toml 找不到 version" }
$current = [version]"$($m.Groups[1].Value).$($m.Groups[2].Value).$($m.Groups[3].Value)"
if ([version]$Version -le $current) { throw "新版號 $Version 必須大於目前的 $current（更新器只認比較新的版號）" }
$cargo = ([regex]$pattern).Replace($cargo, "version = `"$Version`"", 1)
[IO.File]::WriteAllText($cargoPath, $cargo, $utf8)
Write-Host "版號 $current -> $Version"

# 跑測試（順便讓 Cargo.lock 跟上新版號）
Invoke-Native cargo @('test', '--lib')

# —— commit、tag、push（atomic：main 與 tag 要嘛一起上去，要嘛都不上）——
Invoke-Native git @('add', 'Cargo.toml', 'Cargo.lock')
Invoke-Native git @('commit', '--quiet', '-m', "release: $tag")
Invoke-Native git @('tag', '-a', $tag, '-m', $tag)
Invoke-Native git @('push', '--atomic', 'origin', 'main', $tag)

Write-Host ""
Write-Host "已推上 $tag。GitHub Actions 會自動 build 並開 release："
Write-Host "  https://github.com/tfluan0606/hachimi-tw/actions"
