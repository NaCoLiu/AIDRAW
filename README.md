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
- 图片编辑：切换到「图片编辑」，添加一张或多张 PNG / JPEG / WebP 参考图并输入修改要求；也可以点击生成结果的「编辑此图」。局部修改直接在提示词描述，当前接口不支持有效的 mask。
- 会话：左侧新建和切换；标题旁的铅笔按钮可重命名。删除绘画和删除会话均有二次确认。
- 图片：会话内使用固定 300×300 的 cover 预览，等比填满并居中裁切；点击打开原图。可另存为、复用提示词或继续编辑。
- 任务：后台异步提交，显示排队与生成进度。暂停只停止本地查询；已知任务 ID 可继续查询，应用重启后也可恢复。

每次生成一张图。模型仅有 `gpt-image-2`、`gpt-image-2-vip`、`gpt-image-2.5`；`xhigh` / `max` 仅限 2.5 模型。每个会话可运行一个任务，不同会话可同时生成。旧版 2.5 模型选项自动迁移到 `gpt-image-2.5`，历史绘画记录保持原样。

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

接口测试使用本地模拟服务，不调用付费 API。覆盖异步文生图、多图 multipart 编辑、无 status 的成功响应、结果下载、鉴权错误、任务恢复和本地持久化。

接口参考：<https://docs.rightapi.ai/docs/rc_draw/openai.html>

组件参考：<https://gpui-kit.com/component>
