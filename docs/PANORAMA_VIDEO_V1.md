# 全景视频首版 / Panorama video v1

开发分支 `feat/panorama-video-v1`；不是新版本发布声明。

## 输入和使用

仅支持 FFmpeg 可解码、已拼接、单目、完整 360°×180°、2:1 等距柱状（ERP）的 MP4。不按相机品牌设白名单；不支持双鱼眼、双目、VR180、裁切全景或厂商私有原片。请先使用厂商软件导出标准单目 ERP。

素材输入框与下方识别状态共用边框和一条内部横线。空态显示“请输入视频文件或图片文件夹”，选择视频后显示“正在检测视频类型…”；检测失败可在框内重试。有完整 ERP 元数据且符合支持条件时自动使用全景流程，显示“全景视频”和“已识别完整 ERP 元数据，将自动按全景视频生成”。普通视频显示“将按普通视频处理”。仅有 2:1 尺寸时，在素材框内选择“普通视频／全景视频”，不预选；未选择不能开始生成。手动选择全景不冒称检测到了 ERP 元数据，选择仍可修改。不再提供视频预览或生成后的投影确认弹窗。2:1 普通视频仍可走普通流程，已知不支持的投影不能选择全景。球面 yaw/pitch/roll 只是全局观察方向，不改变完整 ERP 的局部射线；SfM 使用自身世界坐标，不因品牌的观察方向元数据而拒绝。FFmpeg 的 180° 显示旋转仍是完整 ERP；90°/270° 会改变横纵布局，需先规范化导出。

生成前的统一多步检查仅保留图片数量提醒，支持返回、取消和键盘操作；视频类型不再进入弹窗。自动队列开启前需所有有素材的待命任务检测完成、疑似视频均已选择类型，否则保持关闭并定位第一个需处理的任务。开启后新素材检测期间不自动启动后续任务；发现待选择或检测失败的素材时关闭队列，不影响当前运行任务、不跳过该任务，用户处理后需手动重新开启。开启开关本身不启动第一个任务。选择按草稿和素材身份保存在本机；路径、大小或修改时间变化后失效，后端启动前再次校验。

已完成／未完成详情采用同一只读素材框，统一显示“本任务按全景视频处理／本任务按普通视频处理／本任务按图片序列处理”。类型由已有检查点返回的可选 `ProjectTaskDetail.videoProjection` 提供，不重新检测原文件；确实未记录时明确显示未知，导入 Gaussian 项目不显示不适用的视频信息。不修改项目 schema。

训练节点区分“准备训练图片”和实际 Brush 训练。前者显示原全景帧及六面图片真实计数，缓存复用单独写日志，校验完成前不结束；全景总进度分配 60%–66% 给准备、66%–98% 给训练，普通任务保持原映射。未知总量显示处理中，不估造百分比；训练步数和剩余时间仅使用 Brush 的真实数据，切换子阶段清除旧计数。事件的可选 `substage` 用于本机界面，不新增云端统计字段或永久项目阶段。

CLI 示例（指定诊断目录时不会登记到应用列表，也不发送埋点）：

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --bin splatstudio -- generate "D:/video/stitched-360.mp4" --projects-root "D:/diagnostic-projects" --quality fast --video-projection equirectangular
```

`--video-projection` 支持 `auto / perspective / equirectangular`。普通视频流程、已有项目检查点和普通补拍不变。全景项目首版暂不开放高清补拍。

## 实际处理流程

1. FFprobe 检查投影、立体布局、尺寸及旋转。全景抽帧另行流式读取解码帧索引、原始 PTS 和真实时间，不使用平均 FPS 推算时间戳；最多 100 万帧，时间探测输出只保留 4 KiB 诊断尾部。
2. 复用档位初始采样和 Bridge 候选预算，以原全景时刻为选择单位。保存整幅 ERP，而非六面图；Alpha 图和 Mask 保持现有语义。
3. COLMAP 模型 17 `EQUIRECTANGULAR`，参数为 **width,height**，不是焦距。顺序匹配与 Incremental Mapper；局部、全局 BA 显式 `CERES`，包括 Bridge 重建，完全绕过 Caspar。全景专用初始化三角化角度 1°，允许任意方向平移；普通透视参数不变。
4. 保留 `work/colmap/` 球面结果为权威重建。只有最终注册的 ERP 帧转换成 front/back/left/right/up/down 六面；不再匹配或重跑 SfM。
5. 派生 `work/panorama-training/`：六面共享中心，`R_face = Q R_erp`、`t_face = Q t_erp`。相机 x 向右、y 向下、z 向前，像素中心 `(x+0.5,y+0.5)`，主点 `(N/2,N/2)`，`f=N/(2 tan(46°))`。RGB 双线性采样、Mask 最近邻采样共享映射；ERP 左右取模环绕、两极钳制。
6. 六面共享 PINHOLE 相机。初始化三维点的位置、颜色、尺度保持不变；新图像 ID 从 1 编号，二维观测和旧 ERP 轨迹清空，避免悬空引用。
7. 使用独立的[全景训练配方 v1](PANORAMA_IMAGES_V1.md#全景训练配方-v1)：关闭自动优化时快速／均衡／精细上限为 100 万／200 万／300 万；开启时按质量档位与显存预算限制，最高 1,000 万。训练仍为 12,000／30,000／45,000 步，不乘六。结果继续发布 `final.ply`，预览、编辑和导出不变。注册率、缺失时段、Bridge 与效果统计仍按 ERP 帧计数。下文 8,000 步的旧实测记录属于此前配方，不作为新配方验证结果。

### 默认资源预算

| 档位 | ERP SfM 最大宽度 | 每面最大边长 | 六面总像素上限 |
| --- | ---: | ---: | ---: |
| Fast | 2048 | 512 | 1.5 Mi pixels |
| Balanced | 3072 | 768 | 3.375 Mi pixels |
| High | 4096 | 1024 | 6 Mi pixels |

每面 FOV 92°，轻微重叠；面尺寸不超过实际 ERP 宽度的 1/4，不放大源图。逐时刻、逐面写盘，内存中仅保留单幅 ERP、可选 Mask 和一个面。单幅解码预算仍为 512 MiB；2026-10-21 起，抽帧与六面缓存不再使用固定 20 GiB 门槛，也不按预计缓存大小减帧或降分辨率。保留数量、尺寸和整数溢出检查；磁盘写入失败会中止当前处理，不认可不完整缓存。请预留足够磁盘空间。Brush 图像缓存限 2 GiB，GPU 训练另需显存；这些预算不是整个进程/项目占用上限。

### 恢复与映射

- schema v9 增加可选全景检查点；旧项目仍按透视读取，不自动重算。
- 保存源视频 SHA-256、投影、转换版本、尺寸和 FOV。源文件或 SfM 尺寸变化使抽帧及下游检查点失效；仅面参数变化保留 SfM，重建训练缓存。投影类型变化要求新建任务。
- `work/panorama-frame-times.json` 保存选中及候选帧的源索引、PTS 和时间；`work/panorama-training/manifest.json` 将六面图关联到 ERP 图像、时间戳、相机姿态和配置，并保存球面模型指纹。
- 每个 ERP 的六面图作为完整批次原子发布。取消清理当前不完整批次，恢复时复用完整批次；模型文件在全部批次完成后生成，函数成功后才允许训练。
- 转换使用现有训练阶段进度/日志，显示已转换时刻；训练后显示 Brush 真实 step。训练中断仍按原行为重训，不宣称支持优化器状态续训。

## 验证记录（Windows，本地引擎）

`Ballintoy/Input` 是 94 幅真实 ERP JPEG，不含原始 MP4。只读原目录，在 `.cache/panorama-v1/` 合成 10 FPS、9.4 秒的测试 MP4；未复用素材自带 OpenVSLAM 位姿或过时相机参数。

- 打包 COLMAP 4.2.1：实际 CPU 球面特征提取、顺序匹配、Ceres 重建跑通，38/38 时刻注册。透视默认初始化门槛未得到模型，使用全景门槛后成功。
- 应用 Fast：56/56 ERP 时刻注册、7,987 个稀疏点、336 张 480×480 六面图、8,000 步、209,467 个 Gaussian、49,435,829 字节 `final.ply`；约 214 秒（debug，不作为性能承诺）。
- 真实取消/恢复已执行通过：转换两个完整批次后取消，恢复时复用抽帧、特征、匹配、球面重建和已完成六面批次；球面模型内容及修改时间不变。恢复训练产出 219,375 个 Gaussian，GPU 训练不要求位级复现。
- 用实际 `final.ply` 在 Edge WebGL2 渲染六个方向并对照训练图：前后、左右、上下均对应，无缺失半球；快速档的天空、海水和移动人物仍有模糊/拖影，不把几何对齐通过等同于画质完善。截图在 `.cache/panorama-v1/render-1792236975678/`。
- 自动测试覆盖前后半球、六面朝向、共中心、像素中心、Mask/接缝、初始化点保留、无失效轨迹、缓存恢复/损坏重建/取消、三档预算和不支持投影拒绝。真实引擎取消/恢复测试为显式 opt-in：

```powershell
node scripts/test-panorama-runtime.mjs "E:/GaussianSplatting/test/Ballintoy/Input"
$env:OOOSPLAT_PANORAMA_TEST_PROJECT="E:/path/to/completed/diagnostic-project"
cargo test --manifest-path src-tauri/Cargo.toml --lib bundled_panorama_cancel_and_resume -- --ignored --nocapture
node scripts/render-panorama-result.mjs "E:/path/to/completed/diagnostic-project"
```

还需实测：各品牌原生 MP4 元数据及 VFR/非零起始 PTS、原生透明全景视频、长视频磁盘/内存峰值、macOS/Linux 引擎、Balanced/High 最终画质与步数/方向覆盖调优。合成 MP4 跑通不能替代上述实测；不保证动态海水/人物、天空或低视差内容的重建质量。

### 2026-10-18 自动识别与进度改造复核（前一轮弹窗版）

- 前端 354 项及诊断接收端 6 项测试通过；Rust 355 项通过，5 项已有 opt-in 测试默认忽略。Windows 符号链接安全测试使用创建测试夹具所需权限执行，未跳过。
- 前端构建、`cargo fmt --check`、全目标全特性 Clippy、安装资源及许可检查通过。构建仍有现有 PlayCanvas 浏览器外部模块、WebP URL 和大 chunk 提示。
- 自动测试覆盖集中询问、后续新素材询问、取消关闭队列、问题导航与键盘焦点、重复点击、过期快照拒绝、答案跨重启恢复、路径/大小/修改时间身份失效，以及运行任务恰好在开启队列确认期间完成的交接。
- 本轮 Ballintoy 原生引擎验收从已有球面重建检查点开始，真实执行六面生成、取消、校验复用和 Brush 训练；不将复用检查点表述为重新完成了一次从零 SfM，也不将组件测试表述为完整桌面视觉验收。
- 最终真实验收用时 150.33 秒：56 个原全景帧、336 张六面图片、8,000 步训练，产出 223,553 个 Gaussian。球面模型和已完成批次的修改时间保持不变。完整事件回归验证了准备/训练范围、真实计数、单调进度及完成前低于 100%；事件记录位于 `.cache/panorama-v1/last-resume-events.json`。训练随机性导致 Gaussian 数量与此前测试不完全相同。
- 已用实际确认组件、应用样式和 Ballintoy 预览在隔离 Edge 中截图检查；取消/上一步按钮使用与启动按钮一致的高度，沿用现有浅灰与蓝色界面。截图位于 `.cache/panorama-v1/preflight-ui/dialog.png`。仍需人工核对 Windows WebView2 中的主界面完整操作。此前的品牌/平台/素材限制继续适用。

### 同日素材框调整复核

- 当前实现已用素材框内选择替代前一轮投影弹窗；上方“前一轮弹窗版”截图和交互记录仅保留为历史验证，不代表当前界面。
- 前端 367 项、诊断接收端 6 项、Rust 356 项通过；Rust 默认另有 6 项 opt-in 测试（含本轮新增真实素材只读检测测试）。前端构建、格式检查、Clippy、资源和许可检查通过。
- 原截图中的已取消项目目录已不存在，未恢复或修改它；改用现存 `20261018-142000_VID_20200408_160043_00_172-flowstate` 中的实际视频，只读验证 3840×1920、单目 ERP 元数据、无需选择、返回的预览 JPEG 为空，以及历史投影按保存的球面检查点读取。此 opt-in 测试单独执行通过，没有重新生成或训练用户项目。
- 自动测试验证内联选择无默认值、生成按钮禁用、选择可修改、身份变化后失效、检测失败重试、旧分析隔离、历史及共享任务类型展示、队列检测等待与遇到未确认素材时停止。
- 使用实际组件和应用 CSS 在隔离 Edge 检查中文、英文、320px 素材面板及 125% 界面缩放，所有素材框上下部分间隙为 0，未发现横向溢出或残留预览。截图及布局结果：`.cache/panorama-v1/media-input-ui/{zh,en,zh-narrow,en-zoom}.png`、`layout.json`。完整原生 Tauri 窗口人工操作仍需另行核对。

### 2026-10-19 多轮进度与监控修复

- 阶段由生命周期事件推进；日志、心跳、资源采样不再切换阶段或清空监控。每次实际进程执行带操作编号及阶段内执行次数，旧操作事件不能覆盖新操作；这些字段不改变项目 schema 或云端统计。
- 仅对明确完成的图片计数计算特征提取进度。匹配索引、顺序匹配、闭环及局部批次只报告对应子步骤；重建注册图像数不代表重建完成率，反复 BA 或注册尝试不累加成已注册图像。未知阶段进度显示“处理中”。
- 总进度仍采用后端估算值，但界面仅显示百分比；单次执行中不倒退，成功完成前最多显示 99.9%。六面准备及校验仍使用 60%–66%，Brush 使用 66%–98%；“Splat 训练”节点统一按完整 60%–98% 换算，不因准备结束先变为 100% 再归零。
- 活动任务的监控容器保持稳定；没有当前进程快照时显示准备说明与占位值，不回填旧 PID、六面计数或训练 ETA。只有实际 Brush 迭代作为训练步数。
- 新增真实完整生成验收入口：`OOOSPLAT_PANORAMA_VIDEO` 指向现有标准 ERP MP4，执行 `cargo test --manifest-path src-tauri/Cargo.toml --lib bundled_panorama_progress_full_pipeline -- --ignored --nocapture`。测试使用独立本地缓存项目，不登记到应用、不发送统计、不改写原项目；真实事件和结果摘要保存于 `.cache/panorama-v1/progress-e2e-{events,result}.json`，生成结果保留在该目录的独立临时项目中。

- 实测完整新建任务（不是复用 SfM）：现有 3840×1920、14.38 秒 ERP MP4，Fast，86/86 原始画面注册，16,757 个稀疏点，516 张六面图，8,000 步，240,002 个 Gaussian，56,642,087 字节 PLY，约 342.95 秒。原素材、原项目及最终模型未被改写。
- 隔离 Edge 用实际组件、样式和该任务的 2,464 条原生事件分别回放中英文界面，监控容器重建、训练节点名称错误、提前显示 100% 均为零。回放脚本、截图和机器可读结果在 `.cache/panorama-v1/progress-ui/`；它不是 Windows 原生 WebView2 人工目视验收。
- 另行执行真实取消/续跑：两个完整六面批次后取消，恢复复用 SfM 及完整批次，再执行 Brush 训练，86 个 ERP 帧、235,723 个 Gaussian，测试约 223.91 秒；权威球面模型及已完成批次的内容与修改时间保持不变。续跑事件保存于 `.cache/panorama-v1/last-resume-events.json`。
- 自动回归：379 项前端及 6 项诊断测试、构建、Rust 格式检查与 Clippy 通过；Rust 常规测试 357 项通过，7 项资源依赖测试默认忽略。本机缺少符号链接权限的 2 项既有测试明确排除，不改测试来掩盖权限限制。

### 同日进度文案与匹配日志复核

- 实时进程右上角只显示百分比：新运行从 0.0% 开始，未知更新保留本任务、本次运行的最近值，成功后才显示 100.0%。切换任务、重试及续跑不会串用其他运行的显示值；阶段进度未知时仍显示“处理中”。
- 普通重建恢复“正在还原拍摄场景”，实际 Brush 训练统一使用“正在生成高斯泼溅”；可信计数后追加耗时提示。全景重建保留原始画面注册计数，六面准备／校验继续使用独立提示，不作为训练步数。空素材草稿仅显示“新任务”，内部序号和排序不变。
- 主生成、补拍及 Bridge 的匹配调用统一使用匹配解析器。真实日志回放保留全部 86 条 `Indexing image` 输出，不将索引完成率当成阶段完成率。实际无输出满 15 秒时增加系统提示，持续静默每 30 秒最多一次；恢复输出重置计时，退出或取消后停止，提示不推进进度。
- 自动验证：416 项前端、6 项诊断接收端和 365 项常规 Rust 测试通过；前端构建、Rust 格式检查及 Clippy 通过。9 项资源依赖测试默认忽略，真实匹配日志回放另行执行通过；2 项既有符号链接测试因本机权限不足明确排除，未计入通过数。静默提示测试包含真实 15 秒无输出窗口。
- 用此前真实流水线的 2,464 条事件回放中英文生产组件，监控容器重建、错误训练节点名称及提前 100% 均为零。证据保存在 `.cache/panorama-v1/progress-ui/`；匹配回放事件保存在 `.cache/panorama-v1/matching-replay-events.json`。本轮没有重新执行生成或训练，回放不等同于原生 WebView2 人工验收。

## English summary

The monitoring fix uses optional operation identities and actual process-execution counts. Only lifecycle events change phases; logs, heartbeats and resource samples cannot clear or resurrect runtime state. Matching substeps and registered-image counts are not stage-completion percentages. Unknown stage progress is shown as Processing. The Splat training node keeps its name throughout face preparation, validation and training, using one combined stage range. The overall header shows only a percentage, retaining the latest valid value within each task/run and staying below 100% until successful completion; the backend estimate semantics are unchanged.

The caption/log follow-up passed 416 frontend, 6 diagnostic and 365 regular Rust tests, build, formatting and Clippy. Nine resource-dependent tests are opt-in and two existing symlink tests were explicitly excluded for missing fixture-creation privileges. Separate replay retained all 86 real matching-index lines without inventing a stage percentage. Matching silence notices start after 15 seconds, repeat at most every 30 seconds, reset on output and stop on exit/cancellation. Chinese/English replay of 2,464 previously captured native events recorded zero monitor remounts, incorrect training-node labels or premature 100%. No new generation/training run or native WebView2 manual acceptance was performed for this follow-up.

A fresh real Fast pipeline on a 3840×1920, 14.38-second ERP MP4 registered 86/86 source frames and produced 516 faces, 16,757 sparse points, 240,002 Gaussians and a 56,642,087-byte PLY after 8,000 steps (342.95 seconds). Chinese/English Edge replay of all 2,464 native events recorded zero monitor-container remounts, incorrect training-node labels or premature 100%. This does not replace manual native Windows WebView2 acceptance. The 379 frontend tests, 6 diagnostic tests, build, formatting, Clippy and 357 regular Rust tests passed; two existing symlink tests were explicitly excluded because this environment cannot create their fixtures, and seven resource-dependent tests are opt-in.

The separate real cancellation/resume test also passed (223.91 seconds, 86 ERP frames, 235,723 Gaussians), preserving the authoritative spherical model and completed face batches, including modification times. Its events are stored in `.cache/panorama-v1/last-resume-events.json`.

Development-only v1 supports **stitched, monoscopic, full 360×180, 2:1 ERP MP4**. The media picker and its detection status form a single connected box, including an empty prompt, loading state and retry on failure. Confirmed metadata selects panorama automatically. A 2:1 ratio alone never enables panorama: the user must choose ordinary video or full panorama inline before Generate becomes available. Manual selection remains editable and does not claim ERP metadata was detected. There is no video preview or video-type dialog; the extensible preflight remains for image-count warnings. Automatic execution requires every populated standby draft to finish analysis and resolve its type; new unresolved input disables the queue without interrupting the active task or skipping ahead. Users must re-enable it after correction. Choices are stored per draft and source identity, invalidated by path/size/modification changes and checked again before launch. Historical media boxes show the saved processing type without probing source files, using the optional `ProjectTaskDetail.videoProjection` response field without a project-schema change. Stereo, dual-fisheye, VR180, cropped/private formats and quarter-turn layouts cannot enter ERP generation. Static spherical viewing orientation is a global gauge, not a reason to reject standard ERP.

Panorama training preparation reports real panorama/face counts and verified cache reuse, occupies 60–66% overall, and completes only after camera/data validation. Actual Brush training occupies 66–98% and reports real iterations; ordinary progress mapping is unchanged. Unknown totals remain indeterminate, stale counters/ETA are cleared at substage changes, and no cloud telemetry fields or permanent project stages are added.

Whole ERP frames enter SfM with actual decoder indices/PTS/times, camera 17 (`width,height`), sequential matching and Incremental Mapper. Local/global BA explicitly use Ceres, including Bridge. Only registered frames become six co-centered 92° PINHOLE views, with correctly composed rotations **and translations**. RGB/masks share wrap-aware projection. Original sphere models remain authoritative; derived images have fresh IDs, empty observations and no stale ERP tracks; world-space initialization points remain unchanged.

Fast/Balanced/High: ERP width caps 2048/3072/4096, face caps 512/768/1024 without upsampling. Sequential conversion resumes in complete batches. Original Brush steps/densification remain; image cache is 2 GiB. No sixfold step multiplication, GPU projection or panorama reshoots. Draft choices survive restarts; older checkpoints remain perspective.

Windows acceptance used a 9.4-second MP4 synthesized from 94 actual Ballintoy ERP photographs, not supplied poses. CPU smoke: 38/38 registered. Full Fast: 56/56 registered, 7,987 points, 336 faces, 8,000 steps, 209,467 Gaussians, 49,435,829-byte PLY. Native-brand MP4/VFR metadata, native alpha video, long-resource stress, macOS/Linux engines, Balanced/High quality and direction tuning remain separate acceptance work. Unit/mock passes are not claims of visual quality.

2026-10-18 follow-up: 354 frontend, 6 diagnostic receiver and 355 Rust tests passed (5 existing opt-in tests excluded by default), as did the build, formatting, all-target/all-feature Clippy, resource and license checks. Real Ballintoy acceptance for this change resumes a saved spherical checkpoint and exercises face preparation, cancellation/reuse and Brush training; it is not a fresh SfM run. The real dialog component/styles were screenshot-checked in isolated Edge with a Ballintoy preview; full manual Windows WebView2 interaction remains unverified.

Final native acceptance took 150.33 seconds: 56 ERP frames, 336 faces, 8,000 steps and 223,553 Gaussians. The authoritative sphere model and completed cache batches were unchanged. Captured events verify separate preparation/training ranges, real counts, monotonic overall progress and no 100% before completion. Gaussian counts vary with training randomness.

The later inline-picker revision passed 367 frontend, 6 diagnostic and 356 Rust tests, build, formatting, Clippy, resource and license checks. A separately executed read-only real-input test confirmed 3840×1920 ERP metadata, empty preview bytes and saved panorama processing from an existing project. No user project was regenerated. Isolated Edge checks of Chinese/English, a 320px media panel and 125% interface scaling found zero gaps and no horizontal overflow. Full native Tauri manual interaction remains unverified.
