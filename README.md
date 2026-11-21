# PomBot

单用户、自托管的私人 AI 助手：手机通过局域网安全连接自己的电脑，由电脑保存数据并调用你配置的模型。当前版本支持设备配对、自动发现、流式对话、断线重连和消息历史；尚不包含远程界面操作。

## 准备模型

默认配置文件位于 `%APPDATA%\PomBot\config.toml`。首次启动后创建该文件，内容可参考根目录的 `config.toml.example`：

```toml
[server]
host = "0.0.0.0"
port = 8765

[llm]
default = { base_url = "https://api.openai.com/v1", api_key = "你的密钥", model = "gpt-4o-mini" }
```

也可以把 `.env.example` 复制为同一目录下的 `.env`，通过 `POMBOT_LLM_BASE_URL`、`POMBOT_LLM_API_KEY` 和 `POMBOT_LLM_MODEL` 配置。`.env` 中的值优先。模型服务必须兼容 OpenAI 的 `/chat/completions` 和 SSE 流式响应。本地 Ollama 可使用 `http://127.0.0.1:11434/v1`，API Key 留空。

API Key 只保存在 PC，不会发送给手机。不要把真实配置文件或 `.env` 提交到版本库。

## 构建并运行 PC 端

需要 Windows 和 stable Rust：

```bash
cd desktop
cargo run --release --bin pombotd
```

程序会显示托盘图标并打开配对页面，默认地址为 `http://127.0.0.1:8765/pairing`；修改 `[server].port` 或 `POMBOT_PORT` 后以实际端口为准。`Alt+Space` 或托盘菜单可重新打开配对页。若 Windows 弹出防火墙提示，只允许“专用网络”，不要开放到公用网络。

数据默认保存在 `%APPDATA%\PomBot\pombot.db`。可通过 `POMBOT_DATA_DIR`、`POMBOT_CONFIG` 和 `POMBOT_PORT` 覆盖默认路径与端口。

## 构建 Android App

需要 Node.js 22+ 与 pnpm。局域网发现使用 Android NSD，因此不能用 Expo Go，必须构建开发包或 APK：

```bash
cd mobile
pnpm install
npx eas-cli@latest login # 首次使用时登录自己的 Expo 账号
npx eas-cli@latest build -p android --profile preview
```

本地已安装 Android SDK 时，也可运行 `npx expo run:android`。EAS 云构建需要你自己的 Expo 账号登录。Android 模拟器通常只能发现模拟器内部的 mDNS 服务，局域网验收请使用真机。

## 配对与对话

1. 让手机和 PC 连接同一 WiFi。
2. 在手机上扫描 PC 配对页的二维码；也可选择自动发现的电脑并输入 6 位配对码。
3. 核对手机和 PC 上显示的四个短词，一致后在手机上确认。
4. 在对话页发送消息，回答会逐段显示。断线后 App 自动重连并恢复最近 100 条消息。

配对页、二维码及配对状态只允许 PC 本机访问。配对码 5 分钟过期，连续错误后会刷新。设备 token 只以哈希形式保存在 PC，手机使用 Android Keystore 加密保存 token 和 Ed25519 私钥。

配对提示无法连接时，先用手机浏览器访问 `http://<电脑局域网IP>:<端口>/health`。无法打开时检查 Windows 防火墙和路由器的 AP/客户端隔离；能够打开但 App 仍无法连接时，确认手机安装的是包含当前配置的最新 APK，而不是旧构建或 Expo Go。

当前通信面向可信的家庭局域网，不提供外网访问或公共中转。解除手机配对只会删除手机凭据；如需在 PC 注销设备，目前可关闭程序后删除数据库中的对应设备记录。

## 开发检查

```bash
cd desktop
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
cargo test

cd ../mobile
pnpm typecheck
pnpm lint
```

修改 `shared/contract/protocol.json` 后，运行 `./scripts/gen-contract.sh`，并同时提交 Rust 与 TypeScript 生成文件。CI 会检查生成结果是否漂移。

## 许可

代码采用 MIT。PomBot 名称、绒球形象与美术资源不随代码授权；代码中的几何占位绒球仅用于产品占位，不代表其他素材获得相同授权。
