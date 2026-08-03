# 把 promo-footage.html 打包成可以整个 copy 走的独立文件夹。
#
# 源页面引用的是仓库里的 logo（../runtime/icons、../ui/assets），这样 logo 更新
# 时页面自动跟着变；但那些相对路径出了仓库就断了。这个脚本把资源收进包内，
# 并改写页面里的引用路径。
#
# 输出：brainstrom\promo-kit\（已 gitignore，随时可重新生成）

$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot
$src  = Join-Path $repo 'brainstrom\promo-footage.html'
$kit  = Join-Path $repo 'brainstrom\promo-kit'
$utf8 = [System.Text.UTF8Encoding]::new($false)

if (-not (Test-Path $src)) { throw "找不到源页面：$src" }

# 每次全新生成，避免上一轮的残留文件混进包里
if (Test-Path $kit) { Remove-Item $kit -Recurse -Force }
New-Item -ItemType Directory -Path $kit, "$kit\assets", "$kit\voice" -Force | Out-Null

# ---- 资源：仓库路径 -> 包内路径 ----------------------------------------------
$assets = @(
  @{ From = 'runtime\icons\logo-dark.png';           To = 'assets\logo.png' }
  @{ From = 'ui\assets\dictating-me-wordmark.png';   To = 'assets\wordmark.png' }
  @{ From = 'brainstrom\promo-voice\wake.mp3';       To = 'voice\wake.mp3' }
  @{ From = 'brainstrom\promo-voice\sentence.mp3';   To = 'voice\sentence.mp3' }
)

$missing = @()
foreach ($a in $assets) {
  $from = Join-Path $repo $a.From
  if (Test-Path $from) {
    Copy-Item $from (Join-Path $kit $a.To) -Force
  } else {
    $missing += $a.From
  }
}
if ($missing.Count) {
  Write-Host ''
  Write-Host '缺少这些资源：' -ForegroundColor Yellow
  $missing | ForEach-Object { Write-Host "  $_" }
  if ($missing -match 'promo-voice') {
    Write-Host '  -> 先跑 brainstrom\make-promo-voice.cmd 生成配音' -ForegroundColor Yellow
  }
  throw '打包中止'
}

# ---- 页面：改写引用路径 -------------------------------------------------------
$html = [System.IO.File]::ReadAllText($src, $utf8)

$rewrites = @(
  @{ Old = '../runtime/icons/logo-dark.png';         New = 'assets/logo.png' }
  @{ Old = '../ui/assets/dictating-me-wordmark.png'; New = 'assets/wordmark.png' }
  @{ Old = 'promo-voice/${name}.mp3';                New = 'voice/${name}.mp3' }
)
foreach ($r in $rewrites) {
  if (-not $html.Contains($r.Old)) { throw "页面里找不到待改写的路径：$($r.Old)" }
  $html = $html.Replace($r.Old, $r.New)
}

# 改完之后不该再有任何指向仓库上级目录的引用
if ($html -match '\.\./') { throw '改写后仍存在 ../ 引用，包不是自包含的' }

[System.IO.File]::WriteAllText((Join-Path $kit 'index.html'), $html, $utf8)

# ---- 说明文件 ----------------------------------------------------------------
$readme = @"
DictatingMe 宣传片素材页
========================

双击 index.html 用浏览器打开（Chrome / Edge 效果最好）。

录制
----
1. 按 H 隐藏底部控制栏
2. 按空格开始播放，全片约 30 秒
3. 用录屏工具（OBS / Win+G / ScreenToGif）录 1920x1080 区域

画面是固定 1920x1080 的舞台，会自动缩放适配窗口。
把浏览器窗口拉到接近 1080p 或全屏（F11），录出来最清晰。

控制栏
------
- 下拉框：单独播放某一段，方便重录某个镜头
- 三个开关：音效 / 背景音乐 / 配音，可分别关掉后期自己配

包内容
------
index.html          页面本体，所有动画和音效都在里面
assets/logo.png     应用图标
assets/wordmark.png 手写体字标
voice/*.mp3         配音

音效和背景音乐是 WebAudio 实时合成的，没有外部文件；
背景音乐为原创合成，无版权顾虑。
配音由 edge-tts 合成（zh-CN-XiaoxiaoNeural，语速 +25%）。

整个文件夹可以直接 copy 给别人，不依赖仓库。
"@
[System.IO.File]::WriteAllText((Join-Path $kit 'README.txt'), ($readme -replace "`r`n","`n" -replace "`n","`r`n"), $utf8)

# ---- 汇报 --------------------------------------------------------------------
Write-Host ''
Write-Host "打包完成：$kit" -ForegroundColor Green
Write-Host ''
Get-ChildItem $kit -Recurse -File | ForEach-Object {
  '  {0,-26} {1,8:N1} KB' -f $_.FullName.Substring($kit.Length + 1), ($_.Length / 1KB)
}
$total = (Get-ChildItem $kit -Recurse -File | Measure-Object -Property Length -Sum).Sum
Write-Host ''
Write-Host ('  合计 {0:N0} 个文件，{1:N1} KB' -f (Get-ChildItem $kit -Recurse -File).Count, ($total / 1KB))
