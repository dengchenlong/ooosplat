# 全景图片与快速素材检测

分支：`feat/panorama-video-v1`。这是开发分支说明，不表示已发布到 0.6.1 安装包。沿用现有图片文件夹入口；不支持单张生成、双鱼眼拼接、双目、VR180、裁切全景或私有原片。

## 输入与检测

- 至少两张 JPG/JPEG/PNG，按文件名自然排序，EXIF 方向归一后的尺寸一致。ERP 必须完整、单目、2:1。
- 同时最多四个文件头读取器。JPEG APP1、PNG eXIf/iTXt 中的 EXIF/XMP 共享每文件 64 KiB 预算，压缩 iTXt 解压输出也计入预算；跳过像素载荷。XML 不展开 DTD/外部实体。缺失、冲突、截断或超限元数据不自动确认为 ERP。
- 按 [GPano 规范](https://developers.google.com/streetview/spherical-metadata)核对投影、全幅和裁切尺寸、零裁切偏移。全部图片一致、可验证且符合尺寸要求才自动确认；仅 2:1 要求选择。已知非 ERP、立体和裁切证据禁止启用全景。
- 快速检测不是完整的可解码性校验：损坏的像素载荷可能在准备阶段报错。生成阶段通过实际解码器重新校验 EXIF；无法与已检测尺寸安全对齐的素材拒绝处理，不静默使用错误方向。
- 视频基础信息与投影共用一次 FFprobe 元数据调用，无 `-count_frames`、`-show_frames` 或 FFmpeg 预览提取。真实视频时间戳仍在生成阶段读取。
- `analyze_input` 先返回轻量结果，`probe_and_plan` 后续规划与硬件检查；选择框不等待硬件结果。后端有容量八项的单飞缓存；草稿已有分析也沿用旧视频接口的身份校验契约，缓存淘汰后仍可重规划，新增覆盖超过八份草稿的测试。质量/自动优化变化只规划，不重复头读取或 FFprobe。
- 文件夹身份覆盖规范化目录路径及自然排序后的成员名、大小和修改时间；视频沿用路径、大小和修改时间。重启重新验证身份，前端用草稿修订和路径隔离迟到响应；素材变化清除旧答案。
- 自动队列对图片和视频应用相同屏障：检测中不启动后续任务，未选择不跳过；疑似新素材关闭队列，选择后需手动重新开启。

## 处理路径与初值

| 档位 | ERP 宽度上限 | 六面单面上限 | 视场 |
| --- | ---: | ---: | ---: |
| 快速 | 2048 | 512 | 92° |
| 均衡 | 3072 | 768 | 92° |
| 精细 | 4096 | 1024 | 92° |

不放大源图；ERP 高度为宽度的一半，面尺寸不超过 ERP 宽度的四分之一。全部输入图片参与准备，不使用视频 FPS、采样比例或 Bridge 候选。

1. 图片导入保持原始字节，记录 `source/image-origins.json` 中原路径、大小、修改时间与项目副本名；不改写外部原图。
2. 生成时计算源内容 SHA-256，与 EXIF 方向、准备文件名、档位组成检查点。派生图片规范化 EXIF、等比缩小；RGBA、COLMAP 图片和二值 Alpha Mask 像素对齐。COLMAP Mask 名为完整图片名追加 `.png`。
3. COLMAP 使用共享模型17 `EQUIRECTANGULAR`，参数 width,height；图片默认穷举匹配，超过100张时生成前确认顺序，已按拍摄顺序排列则使用顺序匹配（重叠窗口10），否则仍为穷举匹配。顺序匹配在自动优化开启时沿用档位闭环检测规则。策略保存在任务请求和项目检查点，续跑不重新询问，旧项目缺少记录时仍为穷举匹配；不要求未排序素材伪装成有序。Incremental Mapper、显式局部/全局 Ceres BA、现有 1° 初始化配置不变，不启用 Caspar。
4. 球面重建保留为权威结果。仅已注册图片进入现有六面缓存；同步转换 `R_face = Q R_erp`、`t_face = Q t_erp`，不再匹配或重建。RGB/Alpha 与 Mask 共用环绕映射，Mask 最近邻。
5. 三维初始化点坐标和颜色保持不变，旧 ERP 二维观测与轨迹清空。使用下述全景专用 Brush 配方，不把步数乘六；普通素材配方不变。
6. `manifest.json` 的 `config.imageSources` 连接原文件身份、项目副本、方向和准备图片；每面连接注册图像/相机。图片面没有 PTS 或时间戳字段；旧视频清单缺少 imageSources 时仍按视频解释。
7. 图片参考视角取自然顺序中最早已注册图片的 front 面；历史详情读取可选通用 `inputProjection`，保留旧 `videoProjection`。预览、编辑、定位与导出复用现有流程。

单图解码仍限 512 MiB，不代表整个进程峰值内存。2026-10-21 起，准备副本、Mask 与六面缓存不再受固定 20 GiB 门槛限制；保留尺寸、数量与整数溢出检查，不按缓存估算拒绝或减少图片。逐张处理并在发布前校验；取消/失败清理本次临时目录，不将不完整准备结果写入检查点，请预留足够磁盘空间。源内容或配置变化失效图片、特征、匹配、重建和训练；断点复用先校验准备文件和缓存。图片的训练 Alpha 保持连续值，SfM 的二值阈值 Mask 不再截掉低透明度训练区域。

## 全景训练配方 v1

本节同时适用于全景视频和全景图片。`config/pipeline-optimization.json` 的可选 `panoramaTraining` 配置独立于普通素材；旧外部配置缺少此字段时使用本节默认值。项目 schema 和旧任务已保存的 Brush 参数不改写。

| 参数 | 快速 | 均衡 | 精细 |
| --- | ---: | ---: | ---: |
| 关闭自动优化的点数上限 | 1,000,000 | 2,000,000 | 3,000,000 |
| 开启自动优化的档位总上限 | 1,000,000 | 5,000,000 | 10,000,000 |
| 训练步数 | 12,000 | 30,000 | 45,000 |
| 增密间隔 | 200 | 200 | 200 |
| 梯度阈值 | 0.0018 | 0.0012 | 0.0010 |
| 达标点选择比例 | 0.20 | 0.25 | 0.30 |
| 梯度增密开始／截止 | 0 / 10,000 | 0 / 25,000 | 0 / 38,000 |
| ERP 宽度／单面上限 | 2048 / 512 | 3072 / 768 | 4096 / 1024 |
| 六面视场／SH 阶数 | 92° / 3 | 92° / 3 | 92° / 3 |

学习率、透明度衰减、屏幕大点分裂、方向采样及 Brush 核心不改。显式固定 `--growth-start-iter 0 --sh-degree 3`。截止后仍可能剔除、替换和大点分裂；并非点数冻结。三档保持原图像预算及 2 GiB 训练图像缓存预算，不承诺总资源占用固定。

开启自动优化时，基础点数预算取档位总上限与按质量细分的显存预算较小值，实际基础上限如下：

| 训练设备总显存 | 快速 | 均衡 | 精细 |
| --- | ---: | ---: | ---: |
| 未知／不足 8 GiB | 100 万 | 100 万 | 100 万 |
| 8–不足 12 GiB | 100 万 | 200 万 | 300 万 |
| 12–不足 16 GiB | 100 万 | 300 万 | 500 万 |
| 16–不足 24 GiB | 100 万 | 500 万 | 750 万 |
| 24 GiB 及以上 | 100 万 | 500 万 | 1,000 万 |

这些是待实测校准的预算，不代表设备必定能完成。关闭自动优化使用独立字段 `maxSplatsWithoutOptimization`（默认 100 万／200 万／300 万），不再复用开启时的 `maxSplats` 档位总上限，也不自动降档。旧外部配置缺少关闭上限字段时采用本节关闭默认值。初始化点若更大则提高至初始化点数并记录覆盖，始终不超过 1,000 万；超出硬上限停止，不让 Brush 静默截断。既有任务保存的实际配方不改写。

全景三档在开启自动优化时最多 OOM 重试一次：完整退出进程后将实际上限减半，但不低于初始点数；不能降低则失败。其余训练参数不变，重用 SfM 与六面缓存，从头训练，不伪称恢复优化器。普通素材仍保持原有 High-only 重试规则。全景快照保存版本、档位请求、显存预算、初始化点及实际参数，并在六面准备前持久化，取消／恢复沿用该快照。

只有现有 GPU 启动策略能确定训练设备时，才采用其显存值；不能把多 GPU 场景中 COLMAP 选择的设备显存直接视为 Brush 默认设备的显存。无法确定时按未知预算处理，不影响关闭自动优化后的档位上限。

真实增密按“可见且梯度达标点数 × 比例”选点，并非全部点按比例增长。原 Ballintoy 快速配置只有 34 次梯度增密机会，10,192 初始点的理想梯度增密上界约 354,194，实际为 244,621。均衡新配置提供 124 次机会；忽略剔除且平均约 20.5% 点达标时，数学上具备达到 500 万的空间，不是实际承诺。

真实诊断使用 `node scripts/verify-panorama-training.mjs --dataset <已有 work/panorama-training> [--quality all|fast|balanced|high]`。只读源缓存，输出到独立 `.cache/panorama-training-v1/<时间>-<档位>/`，保留日志、逐轮点数、进程内存和整卡显存采样，并回读 PLY 全部浮点数、点数、SH 和字节长度。实际面尺寸取缓存与档位上限较小值；使用快速档 512 面缓存的对照不等于完整验证 768／1024 面训练。三档诊断关闭自动优化，以验证请求的上限，不修改项目任务快照或已有 PLY。

### English

With Auto Optimize off, panorama-only recipes cap splats at 1M / 2M / 3M; with it on, tier ceilings remain 1M / 5M / 10M, further restricted by VRAM. Steps remain 12k / 30k / 45k; refine interval 200, gradient thresholds 0.0018 / 0.0012 / 0.0010, selection fractions 0.20 / 0.25 / 0.30 and growth stops 10k / 25k / 38k. Growth starts at zero; SH degree remains 3. ERP/face resolution caps, 92° FOV and ordinary recipes are unchanged.

Auto Optimize uses Fast / Balanced / High caps of 1/1/1M for unknown or <8 GiB, 1/2/3M for 8–<12 GiB, 1/3/5M for 12–<16 GiB, 1/5/7.5M for 16–<24 GiB and 1/5/10M for ≥24 GiB. These budgets are not memory guarantees. Initial points can raise the effective cap, but never past 10M. Exactly one OOM retry may halve the cap without dropping initial points; all other parameters are preserved. With optimization off, use the separate 1/2/3M caps and do not retry automatically. Older configurations missing the new off-mode fields use the current manual defaults. Snapshots retain the actual recipe across cancellation/resume; existing snapshots keep their old recipes.

The diagnostic reads an existing cache, writes only a fresh `.cache` output, and records real growth and memory samples. Whole-GPU usage is not isolated training VRAM. Cached 512px faces do not validate full-resolution High quality, and reaching 5M requires a successfully completed real export, not a mathematical growth estimate.

### 上限调整回归（2026-10-19）

本次仅调整上限分配，没有重新训练。自动测试覆盖全部显存边界的三档预算、关闭自动优化时与显存无关的 100 万／200 万／300 万上限、旧外部配置默认值、自定义关闭上限、初始化点保护、OOM 减半及旧全景任务保存上限不改写。前端 439 项和诊断 6 项、Rust 387 项、构建、格式检查及 Clippy 均通过；Rust 14 项按需测试未执行，2 项既有 Windows 符号链接权限用例明确排除。诊断脚本语法检查通过。8 GiB 精细 300 万等新预算仍需完整真实训练，自动测试不证明显存安全或实际点数。

This cap-only revision was not retrained. All VRAM boundaries and qualities, VRAM-independent manual caps, old-config defaults, custom manual caps, initialization protection, OOM halving and saved panorama recipes are covered. Frontend (439), diagnostic (6) and Rust (387) tests, build, formatting and Clippy passed; 14 opt-in Rust tests were not run and two existing Windows symlink-permission cases were explicitly excluded. New budgets, including 3M High on 8 GiB, still require full real training validation.

## 界面文案

仅使用本轮用户确认的中英文白名单；复用现有加载、失败、重试与图片数量提醒。普通图片/全景图片及历史说明不再混用视频措辞。原始图片数用于注册率和准备计数，六面数不作为训练步数。

## 验证记录

2026-10-19，Windows x64，RTX 3060 Ti 8 GiB，本机调试构建。所有测试只使用独立 `.cache` 或临时目录；不登记到日常项目列表，不上传原始路径或模型。

### 首次图片流程验收（旧训练配方）

直接读取 `E:/GaussianSplatting/test/Ballintoy/Input` 的 94 张 3840×1920 JPG，没有合成视频、使用素材附带位姿或提前展开六面。素材没有可完整验证的 GPano，轻量检测正确返回“需要选择”；诊断运行显式选择 ERP。

- 快速档 ERP 2048×1024、单面512、92°；穷举匹配、相机17、局部和全局 Ceres。
- 94/94 注册成功，初始化点10,192，564张六面训练图；Brush 8,000步，244,621个 Splat。
- PLY：`.cache/panorama-images-v1/.tmpBiXbO3/20261019-162950_Input/final.ply`，57,732,174字节（约55.1 MiB）。
- 调试期间该次总耗时1266.117秒，Brush日志训练62.876秒。图片准备约649秒、六面约156秒。该运行在开发期间启动，后续调整了 PNG 缓存编码、原文件映射和图片 Alpha 处理；这些变化另有专项测试及续跑验证，不能把这一耗时当作最终发行构建的性能基准。
- 实际产物自动核验：94个球面相机姿态对应564面；六面共中心最大偏差 `2.67e-15`；10,192个初始化点的 ID、位置、颜色和误差逐条一致；旧二维观测和轨迹清空。图片清单没有 PTS/时间戳。
- 实际参考相机为 `erp_0000000001/front.png`，FOV92°。WebGPU中文1600px及WebGL2英文1100px均加载此真实PLY；从三视图自动定位后位置误差约 `2.22e-8`，方向点积约1。
- 取消/恢复实测通过：取消后未进入训练，重新继续后保留球面模型内容/修改时间以及已经完成的六面文件时间，完成8,000步训练，得到239,475个Splat。训练本身具有随机性，不要求两次Splat数量完全相同。
- 取消验收用例首次复用于图片时暴露视频专用“顺序匹配”和计数单位断言；已经按实际输入类别改成穷举匹配/图片面计数并重新通过。产品实际调用未使用顺序匹配。

产物证据：`.cache/panorama-images-v1/{result,events,artifact-proof,reference-view}.json`；续跑事件 `.cache/panorama-v1/last-resume-events.json`。缓存不纳入提交。

### 新训练配方实测（2026-10-19）

以下实测发生于本次上限调整前，关闭自动优化时分别使用 100 万／500 万／1,000 万的旧请求上限；保留原始结果，不将其视为新 100 万／200 万／300 万预算已完成实测。新版诊断脚本已改为使用独立的关闭上限字段。

RTX 3060 Ti 8 GiB、Windows、本地固定 OOOBrush 1.0.0；94 张 Ballintoy 图片、564 面、10,192 个初始化点，所有模型保持 SH3。本次仅重训，不重跑 SfM、不复用数据集自带位姿。旧项目、球面模型、原六面缓存和 PLY 不覆盖。

| 对照 | 面尺寸 | 训练步数 | 实际导出点数 | PLY | 诊断耗时 | 采样整卡显存峰值 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 新快速 | 512 | 12,000 | 508,269 | 114.4 MiB | 144.952 s | 2,624 MiB |
| 新均衡，固定低分辨率缓存对照 | 512 | 30,000 | 1,398,881 | 314.8 MiB | 782.261 s | 4,682 MiB |
| 精细，原分辨率图片＋相同球面位姿 | 960 | 约 41,000 / 45,000，中断 | 无最终导出；中途最后记录 3,476,124 | 无 | 2,391.989 s，OOM失败 | 7,403 MiB |

快速进程工作集／私有内存采样峰值约 971.2／2191.4 MiB；均衡约 1761.0／4482.4 MiB。整卡显存包含桌面和其他进程，不等于 Brush 独占分配；均衡对照期间存在并行 CPU 缓存生成与短时预览，不作为隔离性能基准。两份 PLY 的全部浮点属性、点数、SH 阶数及文件长度已回读通过。它们均未达到 500 万，不能把配置上限当作实测点数。

精细真实训练在约第41,000步报 `wgpu error: Out of Memory`，随后出现读取计数及未初始化GPU资源的连锁错误，退出码101。工作集／私有内存采样峰值约4314.0／10590.0 MiB。测试关闭自动优化，因此未减点、自动降档或重试，没有最终 PLY，更没有达到500万的成功验收。原分辨率缓存的模型和清单前后SHA-256一致。证据 `.cache/panorama-training-v1/1792404259630-high/{summary.json,high/brush.log,high/result.json}`。

首轮精细未完成，不符合“完成但不足500万”的对照前提，因此没有执行0.0008试验；默认值仍为0.0010。500万和1000万实测仍需更大显存设备等后续条件，不能将单元测试或中途347.6万点视为通过，也不能将这次失败推断为所有8GB设备／素材的硬性最大点数。

512 面精细对照在约 15,655 步人工中止，最后记录 1,231,661 点，仅保留部分增长曲线；退出码1是主动终止，不是 OOM 或完成结果。后续精细验收改用 960 面：从项目保存的原始 3840×1920 图片重新投影，使用相同注册位姿和三维初始化点，重算针孔内参，没有上采样、重新 SfM 或伪造初始化点。这是训练密度验收，不是完整精细档 SfM 重跑。

缓存准备专项测试曾在完成投影后误调用 ERP 专用文件名读取器，已改为通用 COLMAP 读取器并复用已生成图片复验通过。专项测试校验原缓存三个模型文件的 SHA-256 均不变。可复现命令：

```powershell
$env:OOOSPLAT_PANORAMA_DENSITY_PROJECT = '<已有图片项目目录>'
cargo test --manifest-path src-tauri/Cargo.toml --lib --offline native_resolution_density_diagnostic_cache -- --ignored --nocapture
# 完成后读取 .cache/panorama-training-v1/native-cache.json 的 dataset。
node scripts/verify-panorama-training.mjs --dataset '<原分辨率诊断缓存>' --quality high
# 仅在完整精细导出不足500万时，对照一次；不更改默认配方。
node scripts/verify-panorama-training.mjs --dataset '<原分辨率诊断缓存>' --quality high --threshold-control
```

基线证据 `.cache/panorama-training-v1/1792402850180-all/`；原分辨率缓存 `native-faces-nZg1M3/`。快速 WebGL2 实际预览 `.cache/preview-navigation/1792403581734-webgl2-zh/`，均衡 WebGPU `.cache/preview-navigation/1792403971625-webgpu-en/`。修正了浏览器诊断服务器缺少大于256 MiB PLY所需的 HTTP Range 响应；这是测试服务器修复，不是应用预览加载协议改动。

新配方的真实取消／恢复另行通过：准备阶段取消后，保存的版本1配方和12,000步参数不变，恢复时复用球面模型和完整六面批次，完成训练并得到512,006点，诊断总耗时161.29秒。测试逐项断言取消前后快照相同、球面内容和修改时间不变、完整面文件时间不变；临时项目自动清理，日常项目及原有PLY不修改。事件证据为 `.cache/panorama-v1/last-resume-events.json`。

English: The new Fast and fixed-512px Balanced recipes exported 508,269 and 1,398,881 finite SH3 Gaussians, respectively. Neither reached 5M. Their PLYs rendered in real WebGL2/WebGPU production previews (IPC is stubbed). Timing is diagnostic rather than an isolated release benchmark; VRAM samples cover the whole GPU. The partial 512px High run was deliberately stopped to prioritize native-resolution acceptance, not because of OOM. A separate 960px cache uses original RGB, identical registered poses and initialization points, with recomputed pinhole intrinsics; authoritative reconstruction files remain untouched.

The native-resolution High run then failed with an explicit wgpu Out of Memory error near step 41,000 (exit 101). Its last intermediate count was 3,476,124, not a successfully exported model. Sampled whole-GPU peak was 7,403 MiB; process working-set/private peaks were about 4314/10590 MiB. No 0.0008 control was run because the first run did not complete. Neither 5M nor 10M acceptance has passed; larger-memory hardware and further real validation remain necessary. Original cache hashes remained unchanged. A separate production-runner cancellation/resume passed with unchanged recipe snapshots, spherical model and completed face batches, completing 12k steps and exporting 512,006 Gaussians; diagnostic temporary files were then cleaned up.

本次实际可验的是Windows固定引擎、上述两份完成模型的真实浏览器预览、新快照的取消／恢复以及显存不足路径。未完成：更大显存设备上的500万／1000万完整导出、768面均衡完整新建流程、精细新建SfM全流程重跑、全部六面接缝与漂浮点逐项画质评估、macOS／Linux实际训练及原生桌面全交互验收。自动测试、数学上界或中途计数不替代这些实测。

最终常规检查：前端439项＋诊断6项通过，前端构建通过；Rust386项通过、14项资源依赖测试默认忽略，原有2项Windows符号链接权限测试明确排除。原分辨率缓存准备、真实取消／恢复两项忽略测试已另行实际执行通过。`cargo check --all-targets --offline`、格式检查、Clippy `-D warnings`、Brush运行库18项测试、许可证／三平台资源校验及`git diff --check`通过。增加了来自真实OOM日志的分类回归，确保根因不被后续“Failed to read counts”误报为普通读取错误。精细OOM试验本身仍是未通过的真实验收，不包含在“常规测试通过”的结论中。

### 检测耗时与调用数

以下为本机调试测试的首次应用调用及紧接着的缓存命中，不清除操作系统磁盘缓存，不承诺其他磁盘有相同速度。

| 输入 | 首次检测 | 缓存命中 | 首次/缓存命中的头读取或FFprobe |
| --- | ---: | ---: | --- |
| 普通MOV真实素材 | 262.457 ms | 0.076 ms | FFprobe 1 / 0 |
| 完整元数据ERP MP4 | 230.324 ms | 0.092 ms | FFprobe 1 / 0 |
| 100张PNG尺寸一致测试图 | 29.291 ms | 5.589 ms | 图片头 100 / 0；FFprobe 0 |
| 1000张PNG尺寸一致测试图 | 284.134 ms | 56.897 ms | 图片头 1000 / 0；FFprobe 0 |

纯头扫描另测：100/1000个带GPano的JPEG头样例首次17.644/178.819 ms，再读17.246/172.370 ms；这些样例故意不含像素载荷，像素解码会失败，而检测仍可成功，证明检测未借用解码路径。Ballintoy94张真实JPG头扫描首次27.066 ms、再读9.646 ms（这两次为直接扫描，不是接口缓存命中）。生产检测没有预览生成、全文件内容哈希或硬件探测；文件身份仍需枚举/读取成员属性，所以缓存命中并非零I/O。

### 自动测试、界面与限制

- 前端438项测试及6项诊断测试、前端构建、Rust默认功能集377项测试通过（13项按需测试不自动执行，2项需要Windows符号链接权限的既有测试单独跳过）；格式检查、Clippy `-D warnings`、许可证和打包资源校验通过。
- Rust覆盖元数据完整/缺失/冲突/超限、压缩XMP膨胀限制、裁切/立体/非ERP、中文路径、自然排序、EXIF、RGBA/Mask像素对齐、连续Alpha、取消和源变化；前端覆盖类型选择、早于规划呈现、质量重规划、队列屏障和跨重启答案。
- 素材框实际Edge截图：中文/英文、1100px、100%及140%缩放，无横向溢出。截图仅挂载生产组件，不使用原生Tauri IPC。真实PLY渲染脚本同样使用IPC测试桩；二者均不等同于完整原生桌面点击验收。
- 本轮未在macOS/Linux重新训练，未完成这些平台或Windows原生桌面全部交互的人工验收；未逐档跑均衡/精细、真实透明全景图、所有品牌元数据或复杂接缝场景。
- 普通图片/视频及全景视频的共享路径有自动回归，实际普通视频与全景视频完成了检测验证；本轮没有重新对三者各跑一遍完整训练。不把这些单元测试当作真实生成效果的全面验收。

### Reproduce

```powershell
$env:OOOSPLAT_PANORAMA_IMAGES='E:\GaussianSplatting\test\Ballintoy\Input'
cargo test --manifest-path src-tauri/Cargo.toml --lib bundled_panorama_images_full_pipeline --offline -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib fast_folder_detection_cache_benchmark --offline -- --ignored --nocapture
$env:OOOSPLAT_PANORAMA_TEST_PROJECT='<diagnostic-project>'
$env:OOOSPLAT_PANORAMA_RESUME_REUSE_FACES='1'
cargo test --manifest-path src-tauri/Cargo.toml --lib bundled_panorama_cancel_and_resume --offline -- --ignored --nocapture
node scripts/verify-panorama-images-ui.mjs zh
node scripts/verify-panorama-images-ui.mjs en
node scripts/verify-panorama-image-artifacts.mjs <diagnostic-project>
```

## 本轮主要文件

- 后端：`src-tauri/src/video/image_projection.rs`（检测/准备/预算/映射）、`video/panorama.rs`（图片六面与连续Alpha）、`project/manager.rs`（原文件对应关系）、`pipeline/runner.rs`（图片流水线/计数/复用）、`engines/ffprobe.rs`（合并元数据探测）、`commands/mod.rs` 与 `lib.rs`（接口）、`reconstruction/reference_view.rs`（图片首帧参考）、`imports.rs`（补拍限制）。`engines/colmap.rs`只更新新增可选字段对应的测试初始化。
- 前端：`App.tsx`、`generationPreflight.ts`、`taskQueue.ts`、`useProgressMessage.ts`、`MediaSourceStatus.tsx`、`TaskConfiguration.tsx`、`SharedTaskDetail.tsx`、`types/pipeline.ts`、`lib/backend.ts`、`i18n/index.tsx` 及对应测试。
- 依赖：`src-tauri/Cargo.toml`/`Cargo.lock`直接固定已有缓存版本的quick-xml与flate2；没有新增引擎或云端服务。
- 验证：`scripts/verify-panorama-images-ui.mjs`及素材框fixture、`scripts/verify-panorama-image-artifacts.mjs`；复用现有预览浏览器验证脚本。中英文README和本文同步更新。

## English summary

Folders accept two or more same-sized stitched monoscopic full ERP JPG/JPEG/PNG images. Header-only detection uses a four-reader limit and a 64 KiB per-file metadata budget including decompression. Complete consistent GPano metadata auto-confirms ERP; ambiguous inputs require an inline choice. Explicit incompatible metadata cannot be overridden. A single FFprobe supplies video metadata, and replanning reuses cached detection.

Image preparation normalizes EXIF into derived files, preserves RGB/Alpha/Mask alignment, and processes all images without FPS or Bridge sampling. Ordinary and panorama image tasks with more than 100 images ask whether filename order follows capture order before starting. Ordered sets use sequential matching (overlap 10, preset loop detection only with automatic optimization); unordered sets and sets of up to 100 images use exhaustive matching. No answer is preselected. Queue execution waits for the answer; cancellation disables the queue without skipping the task. The strategy is saved with the task and project checkpoint; resumes retain it and legacy projects default to exhaustive matching. Spherical COLMAP retains explicit Ceres BA; registered images alone produce existing 92° pinhole faces. Brush training settings are unchanged. Source identities and face mappings are persisted without invented video timestamps. The first naturally ordered registered image's front face supplies the reference view. Existing videos/projects remain compatible; panorama reshoots stay disabled.

### Image-order confirmation verification (2026-10-19)

- `npm test`: 468 frontend tests and 6 diagnostics tests passed. Tests cover 100/101/500/501 images, ordinary/panorama choices, no default answer, navigation, mutually exclusive options, Chinese/English copy, cancellation/Esc, duplicate clicks, changed configuration, queue waiting/selection/cancellation and IPC forwarding.
- Rust library tests: 411 passed, 17 opt-in tests ignored. Two existing Windows permission fixtures (`unauthorized_paths_and_log_symlinks_are_rejected`, `default_input_directory_rejects_redirected_folders`) were excluded under the restricted environment. New tests cover request persistence across restart, retry immutability, checkpoint round-trip, legacy defaults, saved-policy precedence on resume and matching/loop-detection dispatch; existing COLMAP argument tests cover overlap 10 and loop options.
- `npm run build`, `cargo fmt -- --check` and offline Clippy (`--lib --tests -- -D warnings`) passed. Existing browser externalization/WASM URL/chunk-size build warnings remain.
- No full reconstruction or training was rerun. Automated dialog tests do not replace native desktop keyboard/layout acceptance or real reconstruction timing/quality checks. Existing PNG focal-workaround changes, source images and reconstruction databases were preserved; no commit, push or version change.
