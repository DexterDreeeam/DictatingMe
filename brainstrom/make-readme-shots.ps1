# 用无头 Chrome 给 README 生成配图。
#
# 页面支持 ?still=<场景> 定格模式（见 promo-footage.html 末尾），这里逐个访问
# 并截图，然后自动裁掉四周多余的黑边 —— 舞台是固定 1920x1080，但每个场景的
# 内容高度不同，统一裁剪会切到字幕或留下大片空白。
#
# 输出：docs/media/*.png（进仓库，README 直接引用）

$ErrorActionPreference = 'Stop'

$repo  = Split-Path -Parent $PSScriptRoot
$page  = Join-Path $repo 'brainstrom\promo-footage.html'
$outDir = Join-Path $repo 'docs\media'

$browser = @(
  "${env:ProgramFiles}\Google\Chrome\Application\chrome.exe"
  "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe"
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
  "${env:ProgramFiles}\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser) { throw '找不到 Chrome 或 Edge' }

New-Item -ItemType Directory -Path $outDir -Force | Out-Null
Add-Type -AssemblyName System.Drawing

# 要截的场景 -> 输出文件名（Query 是附加在 ?still=<场景> 后面的额外参数）
# HUD 两个状态分别在 idle（等待唤醒，黄灯）和 wake（正在记录，绿灯）两个场景里
$shots = @(
  @{ Scene = 'brand';   Name = 'hero.png' }
  @{ Scene = 'idle';    Name = 'hud-idle.png' }
  @{ Scene = 'wake';    Name = 'hud-live.png' }
  @{ Scene = 'dictate'; Name = 'dictate.png' }
  @{ Scene = 'inject';  Name = 'inject.png' }
  @{ Scene = 'modes';   Name = 'modes.png' }
)

<#
 .SYNOPSIS
  裁掉图片四周的纯黑边，只保留内容区加一圈留白。
#>
function Trim-Black {
  param(
    [string] $Path,
    [int]    $PadX = 120,
    [int]    $PadY = 70,
    [int]    $Threshold = 20
  )

  $src = [System.Drawing.Bitmap]::FromFile($Path)
  try {
    $w = $src.Width; $h = $src.Height
    $data = $src.LockBits(
      (New-Object System.Drawing.Rectangle 0, 0, $w, $h),
      [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
      [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)

    $stride = $data.Stride
    $bytes  = New-Object byte[] ($stride * $h)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
    $src.UnlockBits($data)

    $top = -1; $bottom = -1; $left = $w; $right = -1
    # 每 2 像素采样一次就够了，全扫 200 万像素在 PowerShell 里太慢
    for ($y = 0; $y -lt $h; $y += 2) {
      $rowBase = $y * $stride
      $rowHit = $false
      for ($x = 0; $x -lt $w; $x += 2) {
        $i = $rowBase + $x * 4
        # BGRA 顺序，只看最亮的通道
        if ($bytes[$i] -gt $Threshold -or $bytes[$i+1] -gt $Threshold -or $bytes[$i+2] -gt $Threshold) {
          $rowHit = $true
          if ($x -lt $left)  { $left = $x }
          if ($x -gt $right) { $right = $x }
        }
      }
      if ($rowHit) {
        if ($top -lt 0) { $top = $y }
        $bottom = $y
      }
    }

    if ($top -lt 0) { throw "整张图都是黑的：$Path" }

    $x0 = [Math]::Max(0, $left   - $PadX)
    $x1 = [Math]::Min($w, $right + $PadX)
    $y0 = [Math]::Max(0, $top    - $PadY)
    $y1 = [Math]::Min($h, $bottom + $PadY)

    $crop = New-Object System.Drawing.Rectangle $x0, $y0, ($x1 - $x0), ($y1 - $y0)
    $dst  = $src.Clone($crop, $src.PixelFormat)
    try {
      $tmp = "$Path.tmp"
      $dst.Save($tmp, [System.Drawing.Imaging.ImageFormat]::Png)
      $dst.Dispose(); $src.Dispose()
      Move-Item $tmp $Path -Force
    } finally {
      if ($dst) { $dst.Dispose() }
    }
    return @{ W = $crop.Width; H = $crop.Height }
  } finally {
    if ($src) { $src.Dispose() }
  }
}

$uri = ([System.Uri]$page).AbsoluteUri
Write-Host ''
Write-Host "浏览器：$browser"
Write-Host ''

foreach ($s in $shots) {
  $out = Join-Path $outDir $s.Name
  Remove-Item $out -ErrorAction SilentlyContinue

  # Chrome 把 "N bytes written to file" 写到 stderr，在 ErrorActionPreference=Stop
  # 下会被当成失败。这里局部放开，改用产物是否存在来判断成败。
  & {
    $ErrorActionPreference = 'Continue'
    & $browser --headless --disable-gpu --hide-scrollbars `
               --force-device-scale-factor=1 --window-size=1920,1080 `
               --screenshot="$out" "$uri`?still=$($s.Scene)&nocap=1$($s.Query)" 2>&1 | Out-Null
  }
  Start-Sleep -Milliseconds 2600

  if (-not (Test-Path $out)) { throw "截图失败：$($s.Scene)" }
  $size = Trim-Black -Path $out
  '  {0,-14} {1,4} x {2,-4}  {3,7:N1} KB' -f $s.Name, $size.W, $size.H, ((Get-Item $out).Length / 1KB)
}

Write-Host ''
Write-Host "配图已生成：$outDir" -ForegroundColor Green
