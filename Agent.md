# Agent.md

本文件是给 AI 编码助手与人类贡献者的工程约定。**动手写代码前先读完本文。**

设计与阶段规划见 `plans/plan.md` 与 `plans/tasks.md`，技术决策记录见 `docs/decisions/`。
这三处均为本地过程文档，已被 `.gitignore` 排除，**不会出现在仓库里**——阅读本文件的人只需按本文执行，不必寻找这些路径。

本文只讲"怎么写"。

---

## 1. 注释规则（强制）

### 1.1 每条注释必须标注作者与时间

**格式**

```
@author <作者> <YYYY-MM-DD>
```

**放置位置**：一段连续注释（视为一条注释）的**第一行**。不是每一行都标，那会产生大量噪音。

**Rust**

```rust
// @author 你的名字 2026-11-19
// 坐标换算：截图统一取物理像素，注入使用绝对归一坐标（0~65535），
// 中间必须经过虚拟桌面原点偏移，否则多显示器下会偏。
fn to_absolute(x: i32, y: i32) -> (i32, i32) { ... }
```

**TypeScript / JavaScript**

```ts
// @author 你的名字 2026-11-19
// 心跳 15 秒续约，60 秒过期；页面切到后台时降频但仍续约。
```

**YAML / JSON / SQL / 脚本**同理，用该语言的注释语法，`@author` 行必须在最前。

### 1.2 作者名取 `git config user.name`

- 自动化写入时使用 `git config user.name` 的值
- 取不到时询问用户，**不要自己编一个名字**，**不要盗用他人的名字**
- 修改别人写的注释时：**保留原作者行，在下方追加自己的行**，不要覆盖

```rust
// @author alice 2026-11-19
// @author bob 2026-11-25 补充：150% 缩放时还需除以 DPI 缩放因子
```

### 1.3 注释里禁止出现阶段名

**禁止**：`M1`、`M2`、`M4`、`Phase 1`、`阶段二`、`里程碑 3` 等任何进度标记。

**原因**：阶段名脱离 `tasks.md` 就失去意义，读者看代码时不知道"M4"指什么；而阶段划分会变，注释却会永久留存，最后变成误导。

**改写示例**

| 错误写法 | 正确写法 |
|---|---|
| `// M4 阶段实现的截图模块` | `// 截图模块：支持窗口与全屏两种范围` |
| `// M1 先不做的功能，留到 M5` | `// 暂不支持多显示器混合 DPI，需要时扩展` |
| `// TODO(M6): 轨迹重放` | `// TODO: 轨迹重放尚未实现，当前每次执行都走模型决策` |

**其他禁止**：`// 新增`、`// 已修改`、`// 优化了性能` 这类变更日志式注释——那是 git 的职责，不是注释的。

### 1.4 注释应该写什么

只写**代码本身看不出来的东西**：

- 为什么这么做（而不是别的做法）
- 某个魔法数字的来源（如 `0xFFFF` 是相对坐标的最大值）
- 已知的系统行为与坑（如"锁屏后 `PrintWindow` 返回全黑"）
- 单位、坐标系、时序假设

**不要**逐行翻译代码，不要写 `// 返回 x` 这种。

---

## 2. 技术栈

### 2.1 已确定，不得擅自更换

| 层 | 选型 |
|---|---|
| PC 端 | **Rust**（单二进制，无运行时依赖） |
| 移动端 | **Expo (React Native) + TypeScript**，仅 Android |
| 契约 | **JSON Schema 文件**（`shared/contract/*.json`，唯一类型源） |
| 数据库 | **SQLite**（`rusqlite`，开启 `bundled`） |
| HTTP / WebSocket | `axum` + `tokio` |
| LLM 调用 | `reqwest`（OpenAI 兼容协议，SSE 流式，多模态图片输入） |
| 截屏 | `windows` crate 直调 `BitBlt` / `PrintWindow` / DWM |
| 键鼠注入 | `enigo`（封装 `SendInput`） |
| OCR | `Windows.Media.Ocr`（`windows` crate 直调 WinRT） |
| 图像处理 / pHash | `image` |
| 托盘 / 热键 | `tray-icon` + `global-hotkey` |
| 序列化 | `serde` + `serde_json` |
| 日志 | `tracing` |
| 推送（可选） | ntfy，自建 |
| 分发 | GitHub Releases + Obtainium |
| 许可证 | **MIT**（代码）；形象与美术资源不随代码授权 |

### 2.2 明确不使用

引入以下任何一项都需要先与用户确认，不得自行添加：

| 禁用 | 原因 |
|---|---|
| **Tauri** | 会拉入 WebView2 与大量依赖，体积从 15MB 涨到 30MB+，抵消 Rust 优势 |
| **Python / Node 子进程** | 单进程架构已经确定，多运行时等于白选 Rust |
| **UI Automation 控件树** | 本项目采用纯视觉路线，只允许截图、OCR、窗口级 OS 信息 |
| **任何运行时托管云服务** | 单用户自托管，运行时不依赖服务的账号、存储、推送；EAS 仅用于开发期构建与签名 |
| **多租户 / 账号 / 密码体系** | 只做设备配对，登录与多用户明确不在范围内 |
| **PaddleOCR 全量框架** | 体积数百 MB，若需精读 OCR 改用 `ort` + 自写后处理 |
| **给形象资源套用代码的 MIT 许可** | `assets/` 下的绒球形象、Logo、图标**保留所有权利**，不随代码授权；新增素材需单独确认许可 |

### 2.3 依赖新增的门槛

新增任何 crate 前确认三件事：

1. 能否用标准库或已有依赖实现
2. 是否支持 Windows 且无额外运行时
3. 编译后体积增加多少（`cargo bloat` 检查）

体积、启动时间、常驻内存是本项目的核心指标，不允许为了少写几十行代码而引入重依赖。

---

## 3. 架构红线

违反以下任意一条的改动，即使功能正确也应拒绝：

1. **单用户，不做多租户。** 不要加 `user_id`、租户隔离、角色权限。
2. **PC 是唯一真相源。** 手机是缓存 + outbox；不要在手机端做权威数据。
3. **只用两种传输通道**：WebSocket（控制 + 同步 + 锁）与 HTTP（产物 Range 下载）。新增第三种通道需先讨论。
4. **能力白名单是硬边界。** 模型只能调用注册过的工具/技能，不得实现"任意执行"。
5. **危险界面强制移交。** 支付、登录、验证码、密码管理器界面命中即中止，这条不可被配置关闭。
6. **一切远程动作可中止、可审计、PC 端拥有否决权。**
7. **纯视觉**：不做 UI Automation 控件语义，只允许截图 / OCR / 窗口标题与矩形 / 光标位置。

---

## 4. 目录结构

```
pombot/
├── desktop/              # Rust（Cargo workspace 根）
│   ├── src/
│   │   ├── bin/pombotd.rs # 入口：托盘 + 服务启动
│   │   ├── core/         # LLM / Agent Loop / Registry / Job / 同步
│   │   ├── gui/          # capture / input / ocr / window
│   │   ├── store/        # rusqlite schema / 迁移 / seq
│   │   ├── net/          # axum WS + HTTP Range + mDNS
│   │   └── contract/     # 由 JSON Schema 生成的类型
│   └── Cargo.toml
├── mobile/               # Expo App
├── shared/
│   ├── contract/         # *.json  JSON Schema（唯一类型源）
│   └── skills/           # 内置技能与轨迹
├── scripts/              # 打包、发版、签名、回归脚本
├── docs/
├── plan.md               # 设计方案
├── tasks.md              # 实施计划单
└── README.md
```

---

## 5. 常用命令

### PC 端

```bash
cargo build --release          # 构建
cargo clippy -- -D warnings    # 必须零 warning
cargo fmt -- --check           # 格式检查
cargo test                     # 测试
```

构建参数已固定在 `Cargo.toml` 的 release profile：

```toml
[profile.release]
opt-level = "z"
lto = "fat"
codegen-units = 1
panic = "abort"
strip = "symbols"
```

### 移动端

```bash
cd mobile
pnpm install
npx expo run:android                              # 本地开发包，需要 Android SDK
npx eas-cli@latest build -p android --profile preview # EAS 云构建 APK
```

### 契约生成

```bash
# 修改 shared/contract/*.json 后，两端都必须重新生成
./scripts/gen-contract.sh
```

**改契约 = 同时改两端生成代码。** CI 会校验"重新生成后无 diff"，漂移会导致构建失败。

---

## 6. 交付检查清单

提交前逐条确认：

- [ ] `cargo clippy -- -D warnings` 通过
- [ ] `cargo fmt -- --check` 通过
- [ ] 新增注释都带 `@author <名字> <日期>`
- [ ] 注释里没有出现阶段名（`M1` / `阶段二` / `Phase 1` 等）
- [ ] 没有 `unwrap()` 处理可预期的错误，错误信息能定位问题
- [ ] 改动涉及坐标、注入、截图时，GUI 回归用例脚本全部通过
- [ ] 改动涉及契约时，两端生成代码已同步
- [ ] 未引入第 2.2 节列出的禁用技术
- [ ] 涉及行为变更时更新了对应文档

---

## 7. 与用户协作的方式

- **架构决策先讨论再动手。** 涉及技术栈、协议、数据模型的改动，先说明方案与代价，得到确认后再写代码。
- **不要为了"让测试通过"而绕过验证。** 回归用例失败时报告真实原因，不要改判据、不要跳过。
- **不要顺手重构未要求的部分。** 需要重构时单独提出。
- **不确定的地方直接问。** 尤其是涉及 Windows 系统行为、DPI、权限这类容易想当然的地方。
