M4 浏览器回归

1. 使用 Chromium/Edge/Firefox 打开 browser-test-page.html，建议窗口尺寸 1280x800。
2. 固定动作脚本：browser-stage0-4.json。
3. 反向用例：browser-negative.json。
4. 浏览器原生 alert/confirm 使用 `{"action":"dialog","accept":true}` 或 `false`，由执行器向当前模态窗口发送 Enter/Esc；仍需在真实浏览器窗口完成端到端验证。
