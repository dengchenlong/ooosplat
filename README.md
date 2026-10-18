<p align="center">
  <img src="assets/readme-logo.svg" alt="OOOSplat Logo" width="180">
</p>

<h1 align="center">OOOSplat</h1>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-Apache--2.0-blue?style=flat" alt="License: Apache-2.0"></a>
  <a href="https://github.com/ooolabdev/ooosplat/releases/tag/0.6.1"><img src="https://img.shields.io/badge/release-v0.6.1-blue?style=flat" alt="Release: v0.6.1"></a>
  <a href="#系统要求"><img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%7C%20macOS-blue?style=flat" alt="Platforms: Windows, Linux, macOS"></a>
</p>

<p align="center">
  <a href="https://github.com/ooolabdev/ooosplat/releases/tag/0.6.1"><strong>下载</strong></a> | <a href="README.md">中文</a> | <a href="README_EN.md">English</a>
</p>

<p align="center">
  <a href="https://trendshift.io/repositories/177239?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-177239" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/177239/daily?language=Rust" alt="OOOSplat — Trendshift Rust 日榜荣誉" width="250" height="55" /></a>
  <a href="https://trendshift.io/repositories/177239?utm_source=trendshift-badge&amp;utm_medium=badge&amp;utm_campaign=badge-trendshift-177239" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/trendshift/repositories/177239/weekly?language=Rust" alt="OOOSplat — Trendshift Rust 周榜荣誉" width="250" height="55" /></a>
</p>

OOOSplat 是一款将普通环绕拍摄视频或图片序列一键转换为 3D Gaussian Splatting 的本地桌面应用。选择素材、项目目录和质量档位后，应用会自动完成画面准备、相机重建、训练与 PLY 发布，并可直接预览、调整和导出结果。

Windows 和 Apple Silicon macOS Alpha 均随应用提供 FFmpeg、FFprobe、COLMAP 和 Brush；Linux 支持目前仅作为 Ubuntu 24.04 LTS x86_64 Alpha 提供。整个生成流程使用本机 CPU 和 GPU，输入素材、工程文件、模型与日志无需上传到云端重建或训练服务。React 界面通过 Tauri 直接调用本机 Rust 后端，普通生成与预览不需要额外启动 HTTP 服务。

查看 [OOOSplat Roadmap](ROADMAP.md) 了解后续规划。

> 0.6.1 加入版本更新提醒、预览移动轴、旋转环、反选快捷键与视角恢复，改进 Gaussian 选择和编辑保存流程；自动优化三档启用视频闭环检测，并修复预览排序与 Planner 效果统计，减少生成期间的界面刷新负担。

## 核心优势

- **一键生成高斯泼溅**：只需选择输入视频或图片序列、项目目录和质量档位，即可自动完成画面准备、COLMAP 相机重建、Brush 训练和 `final.ply` 发布，无需手动拼接命令或配置引擎。
- **全平台兼容**：支持 Windows、macOS 和 Linux，可在三大主流桌面系统上完成本地高斯泼溅生成；具体系统与处理器要求请参阅下方兼容性说明。
- **安全与隐私保护**：素材、抽帧、相机重建数据、高斯模型和日志默认只保存在用户选择的本地项目目录，核心生成流程在本机完成，无需将原始视频、图像或模型上传到第三方重建与训练平台，从而减少数据在网络传输、云端留存和未经授权访问过程中的泄露风险。
- **完全本地化算力**：重建与训练均在用户自己的电脑上运行，不调用远程计算服务。满足要求时 COLMAP 自动使用本机 NVIDIA GPU 加速，否则回退 CPU，过程和数据始终由用户掌控。

## 展示视频

https://github.com/user-attachments/assets/5b9e8cef-4c71-4bfa-ba23-641fcdd37659

## 界面预览

### 创建与管理任务

![OOOSplat 创建新任务与任务列表界面](assets/screenshots/task-workspace.png)

### 高斯泼溅预览与调整

![OOOSplat 高斯泼溅预览与 Transform 调整界面](assets/screenshots/gaussian-preview.png)

## 主要功能

- **一键生成**：支持 MP4、MOV 和 JPG、JPEG、PNG 图片序列，自动完成画面准备、相机重建、训练和 PLY 发布。
- **导入外部模型**：可从“已完成”分组导入标准/压缩 PLY、SOG、SPZ、SPLAT 和 KSPLAT；完整 COLMAP 数据会自动关联，并在满足单相机条件时支持增量高清补拍。
- **三种质量档位**：提供快速、均衡和精细档；自动优化会根据素材与显存选择合适的抽帧、分辨率和训练配置。
- **透明素材**：支持透明 MOV 和 PNG，保留原始颜色与 Alpha，并自动生成 COLMAP 黑白蒙版。
- **多任务管理**：任务列表分为新任务、已完成和未完成，可保存多个草稿、调整顺序并自动执行下一个任务；同一时间只运行一个生成任务，支持取消和断点续跑。
- **补拍**：从已完成项目创建补拍任务，复用原相机和重建数据处理新素材，再用全部已注册画面重新训练；派生项目不覆盖原结果。
- **预览与编辑**：优先使用 WebGPU，不支持时回退 WebGL2；支持模型移动轴和旋转环、区域裁切、点删除、反选与撤销重做。
- **多种导出**：支持标准 PLY、压缩 PLY、SPZ v4、单文件 SOG，以及竖屏或横屏 MP4、单文件离线 HTML。
- **本地运行**：COLMAP 重建与 Brush 训练都在本机完成；满足条件时自动使用 GPU，否则使用可用的兼容路径。
- **跨平台与双语界面**：支持 Windows、Apple Silicon macOS 和 Ubuntu，并可在简体中文与英文之间切换。

### Gaussian 编辑快捷键

- 矩形选择时，左键拖动替换选区，`Shift + 左键拖动`添加，`Ctrl + 左键拖动`移除；中键旋转视角，右键平移，滚轮缩放。
- 黄色高亮表示当前选区；按 `Delete` 或 `Backspace` 删除选中 Gaussian，按 `Esc` 清空临时选区。
- 在矩形、球形或盒形选择中，点击“反选”或按 `Ctrl + I`（macOS 为 `⌘ + I`）切换保留或选中的部分。
- 球形或盒形选择会自动进入正交视图，可切换侧视图、正视图和顶视图，并通过视口控制器或数值面板调整位置与大小。
- 变换面板中的“场景操作轴”可切换“移动／旋转”，再次点击当前选中的按钮即可关闭；左键拖轴操作模型，拖动空白处转动视角。旋转环和面板旋转都围绕模型中心；松开提交一次编辑，`Esc` 取消本次拖轴，操作轴模式保存在本机。
- `Ctrl + Z` 撤销，`Ctrl + Shift + Z` 或 `Ctrl + Y` 重做。模型变换、裁切和删除共享同一历史记录；相机移动和临时黄色选区不计入历史。

## 处理流程

```text
输入视频或图片序列
  │
  ├─ 视频：FFprobe 分析，FFmpeg 按档位目标 FPS 和工作分辨率抽帧；透明视频同步生成 RGBA 画面与 Mask
  ├─ 图片：快速扫描文件头并按文件名自然排序，复制源素材后优先硬链接复用；按需生成 Alpha Mask
  ├─ COLMAP：自动选择 CPU 或 CUDA GPU 提取特征；视频顺序匹配，自动优化开启时含闭环检测，图片穷举匹配
  ├─ COLMAP：增量重建并验证注册率和三维点；自动优化开启时可按需尝试桥接补帧
  ├─ Brush：使用可用 GPU 训练 Gaussian Splats；精细档按显存选择训练配置
  └─ 校验 PLY 后原子发布为 final.ply
```

COLMAP 生成至少一张注册图像和有效三维点后，普通生成任务会继续进入 Brush。注册率低于 80% 时会给出质量警告，不会仅因注册率低而终止；补拍还要求至少一张新增画面成功注册。

图片序列预检只读取 JPG/PNG 文件头，检查数量、尺寸和 Alpha 通道，不提前解码全部像素。“检测到 Alpha 通道”不等于已经找到透明区域，生成时才检查实际透明度并按需启用 Mask。项目仍保留一份源素材副本；项目内的画面优先通过硬链接复用，不支持时回退复制，素材导入和画面准备期间会持续显示进度与日志。

## 系统要求

- Windows 11，x64。
- 支持 WebView2 Runtime。
- 视频导出需要 WebView2 提供 WebCodecs AVC 编码能力；不支持时仍可在“动画”模式播放效果，但“导出视频”会显示不可用原因。
- Brush 训练需要可用的 GPU 图形后端，建议使用独立显卡。
- COLMAP 的 CUDA 加速需要 NVIDIA 显卡、Windows 驱动 580.00 或更高版本，以及 Compute Capability 7.5 或更高版本；不满足要求时程序会自动使用 CPU，无需用户配置。
- 项目磁盘需要容纳源素材副本、输入图像、COLMAP 数据、Brush 中间文件和最终 PLY。长视频、大型图片序列或精细档位可能占用大量空间。
- 安装模式为整机安装，安装时可能需要管理员权限。

Windows 内置的 COLMAP 使用同时支持 CPU 与 CUDA GPU 的构建，运行前会自动选择可用后端；Brush 训练使用可用图形后端，二者的 GPU 检测与运行机制相互独立。

### macOS 15+ Alpha（仅限 Apple Silicon）

> 当前交付为未签名、未公证的 `.app`/`.dmg` Alpha，仅支持 M1 或更新的 Apple Silicon Mac，不支持 Intel Mac 或 Universal Binary。

- 内置原生 arm64 FFmpeg 8.1.2、独立 FFprobe、COLMAP 4.2.1 Ceres CPU CLI-only 和 OOOBrush ooo-v1.0.0 CLI。
- 用户不需要安装 Homebrew，也不会回退到 Homebrew 或系统 `PATH` 中的同名程序。
- COLMAP 固定使用 CPU；Brush 独立选择可用的 Metal 图形后端，界面会明确显示该原因。
- 首次打开未签名版本时，macOS Gatekeeper 可能阻止启动。确认应用来自可信来源且未被修改后，可按 [Apple 官方打开说明](https://support.apple.com/zh-cn/102445)，在“系统设置 → 隐私与安全性”中选择“仍要打开”；[macOS 15 及以上不再支持右键绕过该提示](https://developer.apple.com/news/?id=saqachfa)。

### Ubuntu 24.04 Alpha（仅限 x86_64）

> 本 Alpha 交付由 Ubuntu 24.04 构建的 x86_64 `.deb` 安装包；不声明支持 Ubuntu 22.04、其他 Linux 发行版或生产环境部署。

- Ubuntu 24.04 LTS，x86_64。
- Brush 支持的图形后端和对应驱动；Brush 官方支持 AMD、Intel 和 NVIDIA GPU。当前端到端验证使用 NVIDIA GPU，CPU-only 软件图形后端尚未验证，但不会被启动检查人为阻止。
- 从源码构建需要 Node.js 22.12+、Rust stable 和 Tauri 2 的 WebKitGTK 开发依赖；安装 `.deb` 的用户不需要这些开发工具。
- Ubuntu 24.04 系统 `ffmpeg`、`ffprobe`；COLMAP 4.2.1 使用校验后随应用分发的自编译版本，不使用 apt/PATH COLMAP。
- OOOBrush ooo-v1.0.0 Linux x86_64 CLI，由 `npm run setup:engines` 下载并校验。

Ubuntu 依赖安装：

```bash
sudo apt update
sudo apt install -y \
  build-essential curl file ffmpeg \
  libwebkit2gtk-4.1-dev libxdo-dev libssl-dev \
  libayatana-appindicator3-dev librsvg2-dev libdbus-1-dev
```

请为显卡安装可用的 Vulkan 驱动（例如 NVIDIA 专有驱动，或 AMD/Intel 的 Mesa 驱动）。内置 COLMAP 在兼容 NVIDIA 设备上使用 CUDA/Caspar，否则使用 CPU/Ceres；Brush 独立选择图形后端。完全 CPU-only 的软件 Vulkan 后端尚未完成端到端验证。

从 main 分支 GitHub Actions 下载 `OOOSplat-0.6.1-x64-linux` Artifact 后，可执行：

```bash
sudo apt install ./OOOSplat-0.6.1-x64-linux.deb
```

`.deb` 仅通过 Ubuntu 包管理器安装 FFmpeg/FFprobe；锁定的 COLMAP 和 Brush 已包含在安装包中。

## 安装与使用

测试素材：[夸克网盘下载](https://pan.quark.cn/s/1dde892a1324)，可用于测试或体验生成流程。

1. Windows 运行 `OOOSplat-0.6.1-x64-windows.exe`；Apple Silicon Mac 打开 `OOOSplat-0.6.1-arm64-macos.dmg` 并将 OOOSplat 拖入“应用程序”；Ubuntu 24.04 使用 `sudo apt install ./OOOSplat-0.6.1-x64-linux.deb`。
2. 启动 OOOSplat，确认顶栏中的内置引擎状态正常；可使用右上角的 `EN / 中文` 按钮即时切换界面语言。
3. 在右侧“任务列表 → 新任务”中选择一个草稿，或点击“新增”。在左侧“创建新任务”的输入类型下拉栏选择“视频”或“图片”，再选择视频文件或图片序列文件夹。
4. 选择项目根目录；程序会记住上次使用的位置。
5. 选择“快速”“均衡”或“精细”档位。
6. 查看自动检测到的 COLMAP 加速状态及原因，然后点击“开始生成”。
7. 在左侧查看所选任务的阶段、进度和日志；点击右侧任意任务可切换详情。生成出的项目按状态进入“已完成”或“未完成”，已完成项目可点击“预览”。
8. 在“预览”的“调整”模式中修改模型，变换、裁切和删除记录会自动保存到项目中，视角按项目保存在本机。点击“保存编辑”可确认当前编辑与视角已保存；需要模型文件时，在“导出”菜单直接选择标准 PLY、压缩 PLY、SPZ 或 SOG，无需再次确认格式。调整和动画模式均可使用。
9. 切换到“动画”，在命令栏中间选择“竖屏 / 横屏”，通过“导出 → 导出视频”生成 30 FPS、23 秒 MP4，分辨率分别为 1080×1920 和 1920×1080；调整或动画模式均可导出离线 HTML。导出使用当前编辑结果，可取消，完成后可打开所在文件夹。
10. 如需补充细节，在已完成项目中点击“补拍”，右侧会创建补拍草稿，不再弹出独立配置窗口。在左侧选择新素材，检查通过后点击“开始生成”；使用原项目相同设备、镜头和拍摄参数，并确保新画面与原素材有足够重叠。

使用提示：

- 拖动左右面板之间的分界线可以调整宽度；双击分界线恢复默认比例。
- 点击右下角百分比按钮可缩小、恢复或放大整个界面。
- 新任务草稿会在本机保存，重启后重新检查素材。运行中的任务参数不能修改，但可以切换并编辑其他草稿；不能同时启动第二个任务。
- 新任务可拖动排序。“自动执行下一个任务”默认关闭，开启后当前草稿生成成功时会启动紧邻的下一个有效草稿；失败、取消或遇到尚未准备好的草稿时停止，不会跳过它继续执行。
- 补拍继承源项目的质量档位和自动优化设置。不能重复输入原素材；图片尺寸必须匹配原相机，视频可以按相同宽高比缩小到该尺寸，但不能放大或改变宽高比。缺失重建数据或使用旧版不兼容图片缓存的项目，需要先重新生成。
- 补拍只对新增画面提取特征，在原数据库和模型副本上补充匹配及重建；Brush 使用全部已注册画面完整重训。派生项目继承模型变换，不继承删点或裁切记录。
- 任务页日志保留最近 500 条，完整引擎日志位于项目的 `logs/` 目录。训练资源面板默认隐藏，可在“设置 → 界面”中开启。
- 点击“删除”会回收整个项目目录，包括源视频副本和所有中间文件；如果移入回收站失败，程序不会降级为永久删除。

## 质量档位

“自动优化（实验性）”默认开启。视频会在抽帧时一次性缩放到对应工作分辨率，并保持原始宽高比且不会放大低分辨率素材；实际采样不会超过源视频 FPS 或总帧数。图片序列始终保留全部有效图片和原始输入分辨率，但 COLMAP 与 Brush 仍使用对应档位的处理上限。

自动优化开启/关闭时各档位的完整参数、Bridge/Caspar/OOM 处理逻辑，以及开发环境外部配置覆盖方法见[自动优化参数配置](docs/pipeline-optimization-config.zh-CN.md)。默认参数集中维护在 `config/pipeline-optimization.json`。

自动优化开启时，三个档位的视频顺序匹配均启用闭环检测，复用内置词汇树、无需联网，帮助连接绕拍回到起点的画面。匹配耗时可能增加，且不保证所有素材都能找到闭环；关闭自动优化时不启用。图片序列、高清补拍及 Bridge 匹配策略保持不变。

| 档位 | 视频初始 / 桥接目标 | 视频工作长边 | COLMAP 长边 / 最大特征数 | Brush 训练 |
| --- | ---: | ---: | ---: | --- |
| 快速 | 6 / 9 FPS | 最大 1,600 | 1,200 / 4,096 | 8,000 iterations，最大分辨率 1,600 |
| 均衡 | 8 / 12 FPS | 最大 1,920 | 1,600 / 8,192 | 15,000 iterations，最大分辨率 1,920 |
| 精细 | 12 / 15 FPS | 依据显存为最大 3,200、3,840 或原生分辨率 | 3,200 / 16,384 | 30,000 iterations，依据显存选择最大分辨率和 Splat 上限 |

短视频会尽量保留至少 30 张画面；源帧不足时使用全部可用画面。因此实际初始和候选采样率可能高于表中的目标值，但不会超过源帧率或源素材帧数。

精细档的初始配置按可用显存划分：显存低于 8 GB 或无法读取时，视频和 Brush 最大长边为 3,200；8 GB（含）至 12 GB 之间为 3,840；12 GB（含）以上保留视频原生长边，并让 Brush 使用原生长边。三个精细配置的 COLMAP 图像长边均限制为 3,200。桥接补帧仅在视频初始重建覆盖不足且仍有候选画面时尝试；失败会安全回退到初始重建，不适用于图片序列。精细档在明确检测到显存不足时最多自动降低一次训练配置并重试。

关闭自动优化后，快速、均衡、精细档分别恢复为保留源视频 30%、50%、100% 画面的旧版策略；视频画面长边沿用旧版最大 1,920，Brush 最大训练分辨率分别为 1,200、1,600、2,000，且不额外设置 Splat 数量上限。

## 项目与文件位置

每次生成都会在项目根目录下创建一个独立文件夹：

```text
<项目根目录>\<yyyyMMdd-HHmmss_素材名>\
  final.ply             最终 Gaussian Splatting 文件
  project.json          项目元数据与结果指标
  edit.ply              导出的编辑结果；再次导出 PLY 时安全替换（可选）
  edit.compressed.ply   压缩 PLY 导出（可选）
  edit.spz              SPZ v4 导出（可选）
  edit.sog              单文件 SOG 导出（可选）
  preview.mp4           首次动画预览视频导出（可选）
  preview-2.mp4         后续视频导出自动编号（可选）
  preview-landscape.mp4 横屏视频导出（后续自动编号）
  preview.html          离线查看器（后续 preview-2.html 等自动编号）
  state.json            流水线状态
  source\
    input.<ext>         源视频副本（视频项目）
    images\             规范化的源图片副本（图片序列项目）
  work\
    frames\             抽取或准备后的输入画面
    colmap_frames\      与原始画面内容一致的 COLMAP 专用图片
    masks\              透明素材对应的 COLMAP Mask（按需）
    reshoot-masks\      补拍素材对应的 COLMAP Mask（按需）
    colmap\             COLMAP 数据库与稀疏重建
    brush\              Brush 数据集与训练中间文件
  logs\                 FFmpeg、COLMAP、Brush 等完整日志
```

项目名中的 Windows 非法字符会被清理；发生重名时自动追加 `-2`、`-3` 等后缀。

应用设置和项目索引保存在：

```text
%LOCALAPPDATA%\SplatStudio\settings.json
%LOCALAPPDATA%\SplatStudio\project-index.json
%LOCALAPPDATA%\SplatStudio\tasks.json
%LOCALAPPDATA%\SplatStudio\telemetry.json
```

以上为 Windows 路径。macOS 和 Linux 使用各自的本地应用数据目录。未启动的草稿、界面偏好和预览视角保存在本机 WebView 存储中，不随项目文件迁移。

## 匿名使用统计

OOOSplat 默认开启匿名使用统计，用于了解稳定性和各阶段耗时。可在右上角 **设置 → 隐私** 中随时关闭，关闭后不再发送匿名统计请求。

会发送的内容：

| 字段 | 说明 |
| --- | --- |
| 安装 ID | 首次启动生成的随机 UUID，不读取硬件序列号、MAC 地址或设备指纹 |
| 应用版本、操作系统、CPU 架构 | 例如 `0.6.1` / `windows` / `x86_64` |
| 事件名 | `daily_active`、`generation_started`、`generation_completed`、`generation_failed`、`pipeline_stage_completed`、`planner_evaluation`。`daily_active` 每天最多一次，应用升级后当天会再报一次 |
| 质量档位与输入类型 | 枚举值，例如 `balanced` / `video`；图片序列输入报 `images` |
| 阶段耗时与总耗时 | 毫秒 |
| 基础生成事件的帧数与视频时长 | 分桶值；Planner 效果事件另含实际素材和重建数量 |
| 自动优化效果指标 | 匿名的素材尺寸与数量、规划结果、注册图像与三维点数、桥接补帧、Brush 配置、Splat 数量、PLY 大小和阶段耗时；普通生成、续跑及补拍均记录，关闭自动优化时仍可用于效果比较 |
| Planner 显卡信息 | 厂商枚举和检测到的总显存，不含完整显卡名称或设备序列号；无法检测时留空或标记为未知 |
| 失败阶段与错误码 | 枚举值，例如 `colmap_mapper_failed`；不含原始错误文本 |

匿名统计不会发送的内容：视频、图片、PLY 等任何素材；文件名、路径和项目名称；日志与命令输出；用户名或任何个人信息。

### 主动发送错误报告

生成失败后，可在报错弹窗点击“发送错误报告”，先查看脱敏后的完整内容，再选择“同意并发送”。报告包含错误原因、详细错误、失败时最近最多 200 行／64 KiB 日志、应用版本、系统版本和全部可检测的显卡信息；不包含素材、模型、设备序列号或匿名安装标识，路径、文件名和用户名等会移除。读取失败的信息会留空，不会猜测实际使用的显卡。

这是独立的一次性授权：即使关闭匿名统计，也可以主动发送，不会开启统计。取消预览不上传；发送失败仅允许手动重试，成功后显示报告编号。本地报告及预览在本次应用运行期间有效，没有查看时限，关闭应用后清除；云端收到的报告保留 30 天。接收服务尚未部署或网络不可用时，会明确提示发送失败，不会假报成功。服务端部署说明见 [错误报告接入说明](server/diagnostics/README.md)。

## 内置引擎

| 引擎 | 固定版本/构建 | 用途 |
| --- | --- | --- |
| FFmpeg / FFprobe | Windows x64 8.1 系列 LGPL shared 月末锁定构建（n8.1.3-9-g29e619e767）；macOS arm64 8.1.2 LGPL shared；Ubuntu 系统包 | 视频分析与抽帧 |
| COLMAP | 4.2.1 同一 commit；Windows/Linux CUDA + Caspar/Ceres；macOS Ceres CPU | 特征、匹配和相机重建 |
| Brush | OOOBrush ooo-v1.0.0；Windows/Linux x64、macOS arm64 | 无界面 Gaussian Splatting 训练与 PLY 导出 |

Windows、Ubuntu 和 macOS 的来源与校验策略分别记录在 [`engines/manifest.json`](engines/manifest.json)、[`engines/manifest.linux.json`](engines/manifest.linux.json) 和 [`engines/manifest.macos.json`](engines/manifest.macos.json)。大型引擎文件不会提交到 Git；开发者通过 `npm run setup:engines` 恢复本地运行时。Release 打包前会校验来源、哈希、架构、动态库闭包和 Brush CLI 参数。

第三方许可和通知位于 [`licenses/`](licenses/)：

- FFmpeg：Windows 构建为 LGPL-3.0-or-later，macOS 构建为 LGPL-2.1-or-later；均禁用 GPL/nonfree 组件。
- COLMAP：BSD-3-Clause。
- Brush：Apache-2.0。

## 本地开发

### 下载 COLMAP 开发包

Windows 和 Ubuntu 使用 `npm run setup:engines` 下载并校验 COLMAP 和 Brush。Windows 还会恢复锁定的 FFmpeg/FFprobe，Ubuntu 使用系统 FFmpeg/FFprobe；macOS 按下方 Apple Silicon 开发流程准备运行时。

- COLMAP 从 [**ooosplat-colmap**](https://github.com/ooolabdev/ooosplat-colmap) 下载。
- Brush 从 [**OOOBrush**](https://github.com/ooolabdev/OOOBrush) 下载。

如需手动安装，请从两个仓库的 Releases 下载对应平台的包。COLMAP 解压后的内容放到：

- Windows：`engines/colmap/`，可执行文件为 `bin/colmap.exe`。
- Linux：`engines/linux/colmap/`，可执行文件为 `bin/colmap`。
- macOS：`engines/macos/arm64/colmap/`，可执行文件为 `bin/colmap`。

完整保留运行库、许可证和元数据，不要多套一层目录。`npm run dev:local` 和 `npm run build:local` 也会准备并校验锁定版本的 OOOBrush。

详细目录结构和校验方式见 [引擎开发包说明](engines/README.md)。

### Windows 开发环境

- Node.js 22.12 或更高版本。
- Rust stable，目标为 `x86_64-pc-windows-msvc`。
- Visual Studio 2022 Build Tools，包含 Desktop development with C++。
- Tauri 2 所需的 WebView2 开发/运行环境。

安装依赖并启动开发模式：

```powershell
npm install
npm run setup:engines
npm run tauri -- dev
```

### Ubuntu 24.04 Alpha 开发

安装上面的 Ubuntu 系统依赖、Node.js 和 Rust 后：

```bash
npm ci
npm run setup:engines
npm run verify:engines
npm run verify:licenses
npm run tauri -- dev
```

也可以在仓库根目录运行 `./scripts/start-app-linux.sh`，或使用 `npm run start:app:linux`。启动脚本会先校验本机引擎和许可映射，仅在源码更新时重新构建 Release 可执行文件。

Ubuntu 24.04 Alpha 的 `setup:engines` 安装校验后的 COLMAP 到 `engines/linux/colmap/`，并安装 Brush；FFmpeg/FFprobe 保持为系统软件包。Tauri 将 COLMAP 和 Brush 打入 `.deb`，不再声明系统 COLMAP 依赖。

Ubuntu 24.04 Alpha 自动检查位于 `.github/workflows/ubuntu.yml`。普通 GitHub runner 会执行前端、许可映射、Rust、Clippy、FFmpeg 集成、`.deb` 构建、包结构校验并上传安装包与 SHA-256；Brush 端到端验证需要具有可用图形后端的主机或自托管 runner。目前完整流水线仅在 NVIDIA 主机上验证，欢迎补充 AMD、Intel 和软件 Vulkan 的测试结果。

### macOS 15+ Apple Silicon Alpha 开发

在 Apple Silicon Mac 上安装 Node.js、Rust 和 Xcode Command Line Tools 后：

```bash
npm ci
npm run setup:engines
npm run verify:engines
npm run verify:licenses
npm run tauri -- dev
```

上述下载启动流程要求 macOS 混合运行时已完成哈希锁定。当前 `engines/manifest.macos.json` 的分发哈希仍为空，常规 `setup:engines` 会明确拒绝安装；首次本地开发需先按 [macOS CI 工作流](.github/workflows/macos.yml) 构建并验证 FFmpeg/Brush 归档，COLMAP 则从独立的 `ooosplat-colmap` 锁定 Release 安装。该限制针对源码开发的运行时下载，不是使用已经包含引擎的安装包。

Homebrew 用于源码构建前的 `npm run setup:build-deps:macos`，之后执行 `npm run build:engines:macos`。macOS CI 会验证本次构建的本地归档后生成未签名应用和 DMG；若要启用清单指定归档的常规下载，应先审核并填入归档与完整性文件的哈希。

### 测试与检查

```powershell
# 前端测试
npm test

# 前端生产构建
npm run build

# Rust 测试
cargo test --manifest-path src-tauri\Cargo.toml

# Rust 静态检查
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets -- -D warnings

# 校验内置引擎版本、哈希和 COLMAP CUDA 运行时
npm run verify:engines

# 校验第一方许可、第三方通知及安装包资源映射
npm run verify:licenses
```

### 生成 Windows 安装包

```powershell
npm run package:windows
```

NSIS 安装包输出到：

```text
dist-artifacts\OOOSplat-0.6.1-x64-windows.exe
```

首次构建前必须运行 `npm run setup:engines`。`beforeBuildCommand` 会自动执行引擎校验和前端生产构建，但不会在打包过程中隐式访问网络。

## CLI

仓库同时提供 `splatstudio` 诊断 CLI：

```powershell
# 检查所有内置引擎
cargo run --manifest-path src-tauri\Cargo.toml --bin splatstudio -- health

# 读取视频信息
cargo run --manifest-path src-tauri\Cargo.toml --bin splatstudio -- probe "D:\Videos\orbit.mp4"

# 查看抽帧计划但不写出画面
cargo run --manifest-path src-tauri\Cargo.toml --bin splatstudio -- plan "D:\Videos\orbit.mp4" --quality balanced

# 单独抽帧
cargo run --manifest-path src-tauri\Cargo.toml --bin splatstudio -- extract "D:\Videos\orbit.mp4" "D:\Frames" --quality fast

# 运行完整流水线（自动检测并选择 COLMAP GPU 或 CPU）
cargo run --manifest-path src-tauri\Cargo.toml --bin splatstudio -- generate "D:\Videos\orbit.mp4" --projects-root "D:\Splat Projects" --quality balanced

# 图片序列使用同一命令，传入文件夹即可
cargo run --manifest-path src-tauri\Cargo.toml --bin splatstudio -- generate "D:\Photos\object" --projects-root "D:\Splat Projects" --quality balanced
```

开发或诊断时可以通过全局参数 `--engine-dir <路径>`，或环境变量 `OOOSPLAT_ENGINE_DIR`，覆盖默认引擎目录。

Linux 的 FFmpeg/FFprobe 保留各自的环境变量与 PATH 发现方式；Brush 默认只使用托管目录 `linux/brush/brush_app`，显式 `OOOSPLAT_BRUSH` 诊断覆盖也必须支持新 CLI，不再自动回退 PATH。COLMAP 只使用托管目录 `linux/colmap/bin/colmap`，不接受单独的 `OOOSPLAT_COLMAP` 或 PATH 回退。

## MCP（可选）

在“设置 → MCP”中开启后，Codex 等本机 AI Agent 可以创建、启动、查询和取消任务，并读取日志；这些任务会同步显示在应用中。服务默认关闭，仅监听 `127.0.0.1`，无需 token。

连接与配置示例见 [MCP 文档](docs/mcp-v1.md)。

## 常见问题

### 如何让 COLMAP 使用显卡？

无需手动选择。Windows 和 Ubuntu 会检查锁定的 COLMAP CUDA 运行时、NVIDIA 驱动版本和显卡 Compute Capability，满足要求时自动使用 GPU 加速特征提取与匹配。CUDA 13.2.0 的最低兼容要求为 NVIDIA 驱动 580.00、Compute Capability 7.5；Caspar 另需实际 BA 探测通过，失败仅回退 Ceres，不影响可用的 SIFT GPU。检测结果显示在任务配置中，当前任务实际使用的后端以该任务的信息和日志为准；后台检测短暂超时不会把正在运行的任务切换为 CPU。macOS 的 COLMAP 使用 CPU，Brush 独立选择可用图形后端。

### 不同显卡组合会如何处理？

COLMAP 和 Brush 是两个独立阶段：COLMAP 只有兼容的 NVIDIA CUDA 环境才能使用 GPU；Brush 在 Windows 和 Linux 上使用 Vulkan，在 Apple Silicon macOS 上使用 Metal。常见组合按以下方式处理；表中未另行标注平台的组合均指 Windows：

| 平台与显卡组合 | COLMAP | Brush 训练 | 自动优化的精细档显存配置 |
|---|---|---|---|
| Windows：单张 NVIDIA 独显 | 满足驱动和 Compute Capability 要求时使用 CUDA | 明确选择第一张独立 GPU，通过 Vulkan 运行 | 使用该 NVIDIA 显卡的总显存选择 Low、Standard 或 Large |
| Intel 核显 + 单张 NVIDIA 独显 | 使用符合要求的 NVIDIA CUDA | 明确选择第一张独立 GPU，避免误用 Intel 核显 | 使用 NVIDIA 总显存分档 |
| AMD 核显 + 单张 NVIDIA 独显 | 使用符合要求的 NVIDIA CUDA | 明确选择第一张独立 GPU，并仅为 Brush 子进程规避 AMD Switchable Graphics 隐式层 | 使用 NVIDIA 总显存分档 |
| 仅 AMD，或 Intel 核显 + AMD 独显 | 自动回退 CPU | 由 Vulkan 自动选择 AMD GPU；不会禁用 AMD 图形层 | 当前使用保守的 Low 配置 |
| 仅 Intel 核显或 Intel 独显 | 自动回退 CPU | 由 Vulkan 自动选择 Intel GPU | 当前使用保守的 Low 配置 |
| 多张 NVIDIA 显卡 | 选择 Compute Capability 最高的兼容显卡；相同时选择 NVIDIA 索引较小者 | 因 Vulkan 与 CUDA 的设备索引不能可靠对应，不强制 Brush 的设备索引，由 Vulkan 自动选择 | 使用 COLMAP 选中显卡的总显存分档 |
| AMD 独显 + NVIDIA 独显 | COLMAP 使用符合要求的 NVIDIA | 当前会进入单 NVIDIA 优先策略，但跨品牌独显的 Vulkan 排序无法完全保证；建议同时在 Windows 图形设置中把 `brush_app.exe` 设为“高性能”并检查任务日志中的实际适配器 | 使用 NVIDIA 总显存分档 |
| NVIDIA 驱动过旧、型号不兼容或检测失败 | 自动回退 CPU | Brush 仍会尝试通过 Vulkan 自动选择可用 GPU，但不会强制 NVIDIA | 当前使用保守的 Low 配置 |
| 没有可用 GPU 图形后端 | COLMAP 使用 CPU | Brush 可能无法启动；CPU-only 软件图形后端尚未完成端到端验证 | 不适用 |
| macOS：Apple Silicon M 系列（含 Pro、Max、Ultra） | 当前使用 CPU，不启用 CUDA | 自动使用 M 系列芯片的 Metal GPU 和统一内存 | 当前不会按统一内存容量提升显存档，使用保守的 Low 配置 |
| Ubuntu Alpha：兼容的 NVIDIA GPU | 满足运行时、驱动和 Compute Capability 要求时使用 CUDA，否则回退 CPU | 通过 Vulkan 自动选择可用 GPU，不强制 Windows 选卡策略 | 根据兼容 NVIDIA 显卡的总显存选择 Low、Standard 或 Large |
| Ubuntu Alpha：AMD、Intel 或无法检测兼容 NVIDIA GPU | 使用 CPU | 通过 Vulkan 自动选择可用 GPU | 使用保守的 Low 配置 |

上述强制选卡和 AMD 隐式层规避只通过环境变量传给单次 `brush_app.exe` 子进程，不会修改系统环境、驱动设置或其他应用，也不会将 Brush 切换到 D3D12。AMD-only 设备不会应用 AMD 隐式层规避。

如果 Brush 日志显示 `Device Lost`，OOOSplat 会将其与显存不足区分并给出提示，但不会自动降低质量重试。建议接通电源、关闭其他 GPU 高负载程序，并在 Windows“设置 → 系统 → 显示 → 图形”中将 `brush_app.exe` 设为“高性能”。精细档只有在明确识别为显存不足时才会降低一级配置，并且最多重试一次。

M 系列芯片的 CPU 与 GPU 共享统一内存，但当前 Planner 不会把统一内存等同于独立显存。因此，即使是内存较大的 M 系列 Pro、Max 或 Ultra，精细档目前仍采用 Low 配置；这属于保守兼容策略，不代表 Metal 训练只能使用少量统一内存。

### 为什么会出现注册率较低的警告？

注册率较低通常表示可用于重建的连续视角不足。任务仍会继续进入 Brush，但结果质量可能受影响。建议使用曝光稳定、画面清晰、运动连续、视角重叠充分的环绕拍摄视频，避免快速转动、强反光、大面积纯色和运动物体。

### 为什么项目占用空间很大？

每个项目会保留源素材副本、输入画面、COLMAP 数据和 Brush 中间文件，便于诊断和续跑。图片序列在项目内部优先使用硬链接，减少重复物理复制；确认结果后，可通过任务列表中的“删除”将整个项目移入回收站。

### 可以直接在应用中查看 final.ply 吗？

可以。在任务列表中选择已完成项目并点击“预览”，即可在独立预览工作区浏览 `.ply`。“调整”模式支持整体 Transform，以及矩形、球形和盒形 Gaussian 选择；可以非破坏式删除、裁切、反选、撤销和重做。编辑记录会自动保存在项目中；“保存编辑”确认当前编辑与视角已保存，不生成模型文件。在“导出”菜单直接选择模型格式即可生成或安全替换编辑结果，原始 `final.ply` 始终不变。重新打开时恢复编辑与本机保存的视角，黄色选区和撤销历史不跨次恢复。“动画”模式提供 5 秒显现、8 秒冲击波、持续环绕，以及带水印的横竖屏 MP4 和离线 HTML 导出。外部标准/压缩 PLY、SOG、SPZ、SPLAT 和 KSPLAT 可从“已完成 → 导入”导入；不支持 LCC/LCC2、Streamed SOG、2DGS 或普通点云 PLY。

### 超大 PLY 可以预览吗？

大于 256 MiB 的 PLY 使用 Range 分块读取，每次约 1 MB，减少文件传输阶段的整文件缓冲；不会压缩、降采样或丢弃 SH 数据。解析、排序和 GPU 纹理仍需要内存与显存，能否打开取决于模型大小和设备能力，不保证所有大模型都能加载。

## 技术栈

预览的“导出”菜单支持模型、竖屏/横屏视频和单文件离线 HTML。模型导出格式如下：

| 格式 | 用途与体积 |
| --- | --- |
| 标准 PLY | 完整精度，通常最大，适合保存和继续编辑 |
| 压缩 PLY | 文件较小，有损压缩，需查看器支持 |
| SPZ v4 | 文件很小，有损压缩，适合分享 |
| 单文件 SOG | 通常压缩率更高，有损压缩，适合网页展示 |

SOG、SPZ 通常更小，实际大小取决于模型；压缩可能影响细节，不保证固定比例或某种格式始终最小。导出保留全部未删除 Gaussian 和原有 SH 阶数，不降采样；格式自身允许量化和重排，自定义额外属性可能无法保留。参考 [SOG](https://developer.playcanvas.com/user-manual/gaussian-splatting/formats/sog/) 和 [SPZ](https://developer.playcanvas.com/user-manual/gaussian-splatting/formats/spz/) 格式说明。

当前 SPZ 编码器使用约 ±2048 场景单位的坐标范围。含超范围坐标的模型会被拒绝 SPZ 导出并提示改用其他格式，避免坐标溢出；不会自动缩放、居中或删除远端点。

展开“导出”菜单不会扫描模型或生成文件，点击具体格式后才保存编辑并启动导出。标准 PLY 由 Rust 直接写入用于原子发布的临时文件；压缩格式由独立 CPU Worker 按需读取编辑后的虚拟数据视图进行离线编码，不生成、落盘或整份缓存中转 PLY。无删除、裁切和变换时直接分块读取原文件；过滤和变换仅在需要时执行。

SOG 使用 10 次 SH 压缩迭代，可能耗时较长，可取消。三种压缩格式仍需要整模型编码内存，不是完整流式编码：预估额外内存为 `6 × 编辑后等效 PLY 字节数 + 128 × 点数 + 64 MiB`，另计保留点索引占用；预算为系统可用内存的 25% 且最多 512 MiB，检测失败使用 256 MiB。选择格式后若检测到超过预算则停止，请改选标准 PLY；预检不能保证运行时不会内存不足。最终文件仍需磁盘空间，只有校验完成才替换对应导出文件，失败或取消保留旧文件及原模型。

HTML 初始展示完整编辑结果，保留未删除的 Gaussian 与 SH 数据，可离线旋转、平移、缩放并手动播放动画，鼠标操作会暂停播放；支持 WebGPU 优先、WebGL2 回退，不提供模型编辑，也不包含项目路径、源素材、日志或遥测。HTML 体积约为对应 PLY 的 4/3，另含查看器资源，大模型打开仍取决于设备能力。开发和构建命令会预先生成离线查看器资源，单独运行 Rust 前可先执行 `npm run build:html-viewer`。

- 桌面框架：Tauri 2
- 后端：Rust、Tokio
- 前端：React 19、TypeScript、Vite、Zustand
- Gaussian 预览与动画：PlayCanvas Engine、PlayCanvas React
- 视频编码与封装：WebCodecs、Mediabunny
- 原生流水线：FFmpeg / FFprobe、COLMAP、Brush
- 进程树管理：Windows Job Object；Linux/macOS Unix process group

## 许可说明

### 代码许可

OOOSplat 的第一方代码及随附文档以 [Apache License 2.0](LICENSE) 发布。版权声明见 [NOTICE](NOTICE)。

### 第三方组件

FFmpeg / FFprobe、COLMAP、Brush、PlayCanvas 和 Mediabunny 分别适用其自身许可证，不因与 OOOSplat 一同分发而改用 Apache-2.0。直接组件的版本、来源、许可证和许可证正文入口见 [第三方通知](licenses/THIRD_PARTY_NOTICES.txt) 与 [引擎清单](engines/manifest.json)。该清单不表示已经完成 Qt、Boost、Ceres 等传递依赖的完整许可审计。

### 品牌

Apache-2.0 不授予 “OOOSplat” 名称、Logo、图标或其他视觉标识的商标使用权。真实引用、教程、截图、官方未修改版本分发及修改版命名规则见 [OOOSplat Trademark Policy](TRADEMARK_POLICY.md)。

### 生成模型

`final.ply` 等生成结果不会仅因使用 OOOSplat 而自动适用 Apache、GPL、LGPL 或其他随附软件许可证。该说明不判断模型著作权归属，也不授予输入素材或第三方内容的权利；完整边界见 [Generated Outputs](GENERATED_OUTPUTS.md)。
