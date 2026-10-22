# AIDRAW

基于 Rust、GPUI 和 GPUI Kit 的 Windows 桌面图像工作室，按 RightAPI OpenAI 图像接口文档实现。

## 运行

需要 Rust stable（MSVC 工具链）、Visual Studio C++ Build Tools 和 Windows SDK。

```powershell
cargo run
```

打开左下角「连接设置」，填写 RightAPI API Key 并保存。默认 API 地址为 `https://rightapi.ai/v1`。也可通过 `RIGHTAPI_API_KEY` 环境变量提供密钥（启动时优先于系统凭据）。界面保存的密钥使用 Windows 凭据管理器，不写入项目或会话 JSON。

## 使用

- 文生图：填写提示词，选择模型、尺寸、质量与格式，点击「生成图片」。画面尺寸可选预设或「自定义」，分别输入宽高（像素），自定义值会自动保存。宽高须为 16 的倍数、每边不超过 3840、宽高比不超过 3:1，总像素为 655360–8294400；不符合要求时会提示并阻止提交。
- 图片编辑：切换到「图片编辑」，添加一张或多张 PNG / JPEG / WebP 参考图并输入修改要求；也可以点击生成结果的「编辑此图」。局部修改直接在提示词描述，当前接口不支持有效的 mask。
- 会话：左侧新建和切换；标题旁的铅笔按钮可重命名。删除绘画和删除会话均有二次确认。
- 图片：会话内使用固定 300×300 的 cover 预览，等比填满并居中裁切；点击打开原图。可另存为、复用提示词或继续编辑。
- 任务：后台异步提交，显示排队与生成进度。暂停只停止本地查询；已知任务 ID 可继续查询，应用重启后也可恢复。

每次生成一张图。模型仅有 `gpt-image-2`、`gpt-image-2-vip`、`gpt-image-2.5`；`xhigh` / `max` 仅限 2.5 模型。每个会话可运行一个任务，不同会话可同时生成。旧版 2.5 模型选项自动迁移到 `gpt-image-2.5`，历史绘画记录保持原样。

全局字体使用内嵌的 Maple Mono CN Regular / Bold，包含中文，无需安装系统字体。字体来源为 Maple-font v7.9，许可证随程序源码保存在 `assets/fonts/LICENSE-Maple-Mono.txt`。

程序图标同时用于 Windows 可执行文件、窗口和侧栏；素材与生成提示词见 `assets/icons/README.md`。

快捷键：`Ctrl+Enter` 提交、`Ctrl+N` 新建会话、`Ctrl+,` 连接设置。

## 本地数据

默认保存到 `%LOCALAPPDATA%\AIDRAW`：`library.json` 保存会话、提示词和任务信息，`images` 保存生成图片，`references` 保存参考图副本。导入不会修改源文件。删除绘画会清理其生成缓存，已导出图片保留；参考图副本保留以支持其他绘画复用。

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
