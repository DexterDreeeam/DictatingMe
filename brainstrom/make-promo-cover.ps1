# 生成视频封面图（横屏 + 竖屏）。
#
# 和 make-readme-shots.ps1 的区别：封面要保留完整画幅，不能裁黑边——它就是
# 按 16:9 / 9:16 整幅使用的，裁掉留白反而对不上视频比例。
#
# 输出：brainstrom\promo-video\cover-*.png（已 gitignore，随时可重新生成）

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$page = Join-Path $repo 'brainstrom\promo-footage.html'
$outDir = Join-Path $repo 'brainstrom\promo-video'

$browser = @(
  "${env:ProgramFiles}\Google\Chrome\Application\chrome.exe"
  "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe"
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
  "${env:ProgramFiles}\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser) { throw '找不到 Chrome 或 Edge' }

New-Item -ItemType Directory -Path $outDir -Force | Out-Null
$uri = ([System.Uri]$page).AbsoluteUri

$covers = @(
  @{ Name = 'cover-1080p.png';    W = 1920; H = 1080; Query = '' }
  @{ Name = 'cover-vertical.png'; W = 1080; H = 1920; Query = '&vertical=1' }
)

Write-Host ''
foreach ($c in $covers) {
  $out = Join-Path $outDir $c.Name
  Remove-Item $out -ErrorAction SilentlyContinue

  # Chrome 把 "N bytes written" 写到 stderr，Stop 模式下会被当成失败
  & {
    $ErrorActionPreference = 'Continue'
    & $browser --headless --disable-gpu --hide-scrollbars `
               --force-device-scale-factor=1 --window-size=$($c.W),$($c.H) `
               --screenshot="$out" "$uri`?still=brand&cover=1&nocap=1$($c.Query)" 2>&1 | Out-Null
  }
  Start-Sleep -Milliseconds 2600

  if (-not (Test-Path $out)) { throw "封面生成失败：$($c.Name)" }

  # 确认拿到的是整幅，而不是被浏览器按内容裁过
  Add-Type -AssemblyName System.Drawing
  $img = [System.Drawing.Bitmap]::FromFile($out)
  $w = $img.Width; $h = $img.Height
  $img.Dispose()
  if ($w -ne $c.W -or $h -ne $c.H) { throw "$($c.Name) 尺寸是 ${w}x${h}，应为 $($c.W)x$($c.H)" }

  '  {0,-22} {1} x {2}   {3,6:N1} KB' -f $c.Name, $w, $h, ((Get-Item $out).Length / 1KB)
}

Write-Host ''
Write-Host "封面已生成：$outDir" -ForegroundColor Green
