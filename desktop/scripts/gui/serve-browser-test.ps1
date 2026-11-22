param(
  [int]$Port = 8765
)
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$page = Join-Path $root 'browser-test-page.html'
if (-not (Test-Path $page)) { throw "测试页不存在：$page" }
$url = "http://127.0.0.1:$Port/browser-test-page.html"
Write-Host "M4 测试页：$url"
Write-Host "请用固定窗口参数启动浏览器：--window-size=1280,800 --window-position=100,100 --user-data-dir=$root\profile --disable-extensions --disable-save-password-bubble"
python -m http.server $Port --bind 127.0.0.1 --directory $root
