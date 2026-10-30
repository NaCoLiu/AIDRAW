# AIDRAW

基于 Rust、GPUI 和 GPUI Kit 的 Windows / macOS 桌面图像工作室，按 RightAPI OpenAI 图像接口文档实现。

## 运行

需要 Rust stable。Windows 使用 MSVC 工具链，并安装 Visual Studio C++ Build Tools 和 Windows SDK；macOS 使用 Apple Silicon、macOS 13 或更新版本，并安装 Xcode / Command Line Tools。

```powershell
cargo run
```

打开左下角「连接设置」，填写 RightAPI API Key 并保存。默认 API 地址为 `https://rightapi.ai/v1`。也可通过 `RIGHTAPI_API_KEY` 环境变量提供密钥（启动时优先于系统凭据）。界面保存的密钥使用 Windows 凭据管理器或 macOS 钥匙串，不写入项目或会话 JSON。

## GitHub Actions 打包

工作流位于 `.github/workflows/build.yml`，推送任意分支或标签、提交 Pull Request 时自动运行，也可在 GitHub 的 **Actions → Build desktop packages → Run workflow** 手动触发。两个平台分别进行格式检查、测试、Release 编译和打包，使用 `Cargo.lock` 锁定依赖。

| 下载产物 | 构建环境 / 目标 | 包内容 |
| --- | --- | --- |
| `AIDRAW-windows-x64` | `windows-2022` / `x86_64-pc-windows-msvc` | `AIDRAW-<版本>-windows-x64.zip`，解压后运行 `AIDRAW.exe` |
| `AIDRAW-macos-arm64` | `macos-15` Apple Silicon / `aarch64-apple-darwin` | `AIDRAW-<版本>-macos-arm64.dmg` 和包含 `AIDRAW.app` 的 ZIP |

构建成功后，在对应 Actions 运行页面的 **Artifacts** 下载，保存 30 天。每个平台同时提供 `SHA256SUMS-<平台>.txt`，包内带使用说明和 Maple 字体许可证。工作流只上传构建产物，不自动创建 GitHub Release。

Windows 便携版静态链接 MSVC C 运行库。macOS 的 DMG 内含应用和「Applications」快捷方式，可拖入安装；应用使用现有图标生成 `.icns`，最低系统版本为 macOS 13。macOS 包使用 ad-hoc 签名，尚未使用 Apple Developer ID 签名或公证，首次打开可能需要在「系统设置 → 隐私与安全性」允许打开。

本机复现打包需要 Python 3.11 或更新版本，并在对应系统执行：

```powershell
# Windows x64（PowerShell）
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --locked --release --target x86_64-pc-windows-msvc
python scripts/package.py --target x86_64-pc-windows-msvc
```

```bash
# macOS Apple Silicon
export MACOSX_DEPLOYMENT_TARGET=13.0
rustup target add aarch64-apple-darwin
cargo build --locked --release --target aarch64-apple-darwin
python3 scripts/package.py --target aarch64-apple-darwin
```

输出在 `dist/`。字体和界面资源内嵌于程序，不需要另外安装字体或复制资源目录。

## 使用

- 文生图：填写提示词，选择模型、尺寸、质量与格式，点击「生成图片」。画面尺寸可选预设或「自定义」，分别输入宽高（像素），自定义值会自动保存。宽高须为 16 的倍数、每边不超过 3840、宽高比不超过 3:1，总像素为 655360–8294400；不符合要求时会提示并阻止提交。
- 批量处理：默认模式。一次多选 PNG / JPEG / WebP 图片，输入统一处理指令，再点击「处理这 N 张图片」。数量不设固定上限，可用于更换背景、调色、修图、风格转换或其他自定义操作；每张原图单独请求、单独保存结果。例如选取 42 张图，输入「将背景替换为蓝色渐变，保留主体」，会建立 42 个独立任务。
- 融合编辑：把一张或多张图片一起作为参考，输入修改要求，生成一份结果。也可点击结果卡片的「编辑」。局部修改直接在指令中描述，当前接口不支持有效的 mask。
- 工作台：左侧管理项目，中间以卡片查看结果和队列状态，右侧添加图片、编辑统一指令与输出设置。标题旁的铅笔按钮可重命名；删除任务和项目均有二次确认。未提交图片、模式及提示词随项目保存。
- 快捷指令：提供更换背景、抠图透明底、统一色调、优化画质作为起点，点击后仍可自由修改。抠图指令会选择 PNG，但当前接口文档明确不支持透明背景，不能保证输出透明通道；修改提示词或文件格式不能绕过此限制。
- 图片：等比完整预览，棋盘格衬底方便查看透明区域；点击打开原图，支持保存、复用指令和继续编辑。「导出已完成」将当前项目所有已完成结果复制到所选位置的新目录，文件名保留原图名称并添加唯一标识，格式和透明通道保持原样。
- 队列：全局最多同时处理 3 张，其余自动排队；单张失败不会阻塞后续任务，可按「全部 / 待完成 / 已完成 / 需关注」筛选。「暂停后续」只暂停尚未提交的任务，正在运行的任务继续完成；「暂停查询」仅停止本地查询，服务端仍可能运行。重启后，未提交任务可通过「继续队列」恢复；已知任务 ID 只继续查询，不重复提交，提交结果不明的任务不会自动重试。

每个独立请求生成一张图，批量模式按实际图片数量建立队列。编辑与批量处理使用文档列出的 `gpt-image-2`、`gpt-image-2.5-flare` 或 `gpt-image-2.5-sunburst`；文生图保留 `gpt-image-2-vip`、`gpt-image-2.5` 兼容选项。`xhigh` / `max` 仅限 2.5 系列。不会自动替换已选模型，历史绘画记录保持原样，已知任务 ID 仍可继续查询。

图片编辑使用 `POST /v1/images/edits`，按文档以 JSON `image` 数组传递 Base64 data URL，保持参考图顺序；图片 MIME 按文件内容识别，支持中文文件名。`async` 为布尔值 `true`，`n` 为整数 `1`，不发送 `background: transparent`。文生图仍使用原有请求方式。

连接超时与单次提交、下载请求的总超时均为 120 秒；连接耗时包含在该请求的总时限内。批量处理每张图独立计时，排队和其他图片的耗时不会累计到当前请求；融合编辑的多张参考图在同一次请求中上传，共用 120 秒时限。任务提交成功后，每个任务另有 10 分钟的总查询时限，单次查询最多 30 秒。

提交失败会显示接口阶段、HTTP 状态、服务端错误文字与可用的请求 ID。502 表示网关或上游异常，仅凭状态码无法确定是否已受理；请先核对服务端任务。客户端不会自动重发提交请求，避免重复任务或扣费。更换为文档模型及传图格式属于兼容性修复，不保证消除服务端故障。

全局字体使用内嵌的 Maple Mono CN Regular / Bold，包含中文，无需安装系统字体。字体来源为 Maple-font v7.9，许可证随程序源码保存在 `assets/fonts/LICENSE-Maple-Mono.txt`。

程序图标同时用于 Windows 可执行文件、macOS 应用包、窗口和侧栏；素材与生成提示词见 `assets/icons/README.md`。

快捷键：`Ctrl+Enter` 提交、`Ctrl+N` 新建会话、`Ctrl+,` 连接设置。

## 本地数据

Windows 默认保存到 `%LOCALAPPDATA%\AIDRAW`，macOS 默认保存到 `~/Library/Application Support/AIDRAW`：`library.json` 保存会话、提示词和任务信息，`images` 保存生成图片，`references` 保存参考图副本。导入不会修改源文件。删除绘画会清理其生成缓存，已导出图片保留；参考图副本保留以支持其他绘画复用。

可用 `AIDRAW_DATA_DIR` 环境变量指定其他数据目录。避免同时运行多个使用同一数据目录的实例。会话文件损坏或版本不兼容时会保留原文件并阻止生成和覆盖。

## 开发验证

```powershell
cargo fmt --check
cargo test
cargo build
```

接口测试使用本地模拟服务，不调用付费 API。覆盖异步文生图与编辑出图、多图 JSON 上传及 MIME 识别、42 张独立批量请求与单张失败、不同批量数量、并发调度容量、暂停与重启恢复、结果下载、鉴权错误、502 网关详情与不自动重发、本地持久化和保留透明通道的批量导出。

接口参考：<https://docs.rightapi.ai/docs/rc_draw/openai.html>

组件参考：<https://gpui-kit.com/component>


curl -sS -X POST "https://rightapi.ai/v1/images/generations" \ -H "Authorization: Bearer sk-xxxxx" \ -H "Content-Type: application/json" \ -d '{"model": "gpt-image-2","prompt": "一只戴着太空头盔的橘猫，电影级光影","n": 1,"size": "1024x1024","async": true}'
