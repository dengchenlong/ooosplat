# 多格式模型导出验证记录

## 实现范围

本功能在 `main` 实现。本次解除资源门槛不修改版本、项目 schema 或生成参数。早期验证时全景位于独立分支，目前已合入 `main`。

- 展开菜单直接显示四种格式及说明，不预约导出、不扫描模型、不生成文件；选择格式后才保存编辑并预约对应格式。没有第二层格式选择弹窗。
- Rust 准备可随机访问的编辑后数据视图，不生成完整中转 PLY。无过滤时使用源点数；仅删除时通过位图生成索引；有裁切时扫描位置。不再以编码或索引的内存估算拒绝导出。
- 虚拟字节按需从原文件读取，仅对需要的点应用一次变换及 SH 旋转，使用预缓存属性偏移和栈上计算缓冲。完全无编辑时直接读取原文件；标准 PLY 只写最终格式的原子发布临时文件。
- 浏览器独立 CPU Worker 按需加载固定版本转换器；单次二进制 IPC 最大 1,024,000 字节，全局串行请求。不通过 JSON/Base64 传模型，不在 React 状态中保存模型数据。
- 支持标准 PLY、压缩 PLY、SPZ v4、单文件 SOG（10 次 SH 迭代）。压缩仍需要整模型编码内存，并非完整流式编码。
- 三种压缩格式均取消 512 MiB 固定值、可用内存 25% 比例及 256 MiB 后备值的预检门槛；SPZ 仍检查当前编码器约 ±2048 的坐标范围，禁止溢出后静默发布。越界时等待用户确认，再仅调整导出副本，详见文末。取消资源预检不意味着编码完全流式或内存无限；实际分配失败仍可能导致导出失败，操作系统强制终止或 WebView 内存崩溃不保证可恢复。
- 完整解码压缩输出，检查点数、SH 阶数、渲染模型和属性有效性，再发布。透明度量化至 0/1 对应的无穷 logit 是合法值，不误判为损坏。
- 固定 SPZ 绑定在 Vite 构建时适配为静态闭包，不更改 WASM、不修改 node_modules，不开放 JavaScript `unsafe-eval`。WebP WASM 本地打包；测试使用严格 CSP，仅允许本地请求。

## 早期验证记录

以下为解除门槛之前的历史实测。关于内存预算拒绝的结果不再代表当前行为；2026-10-21 的验证另记于文末。

### 自动测试与构建

- `npm test`：354 条前端用例和 6 条诊断接收端用例通过。
- `npm run build`：通过；保留转换器/引擎的 Node 模块外置、默认 WebP URL 和大 chunk 警告。本地 WebP URL 显式覆盖已通过浏览器验证。
- 提交前复核 `cargo test`：348 通过，6 条显式手动测试跳过；下文记录的大文件与原生视图浏览器测试已另行执行通过。
- `cargo fmt --check`、`cargo clippy --all-targets --all-features -- -D warnings`：通过。
- `node scripts/verify-bundle-resources.mjs`、`npm run verify:licenses`：通过，包含三平台资源映射检查。
- 虚拟字节与现有导出器逐字节比较，覆盖连续、随机、重复、跨行及跨头部读取，删除、球/盒裁切、反向裁切、旋转、缩放、SH 及仅应用一次变换；检查准备阶段没有创建模型文件。
- 覆盖源文件变化、零点、非法令牌、顺序/大小限制、取消、迟到响应、重复开始、重试、修订冲突、内存预算、已有文件保护及模拟不完整写入；发布阶段不受迟到编码进度影响。菜单测试覆盖四种格式单次触发、展开零导出调用、禁用、中英文和键盘导航。

### 原生虚拟视图 → 真实浏览器编码与回读

使用本机无头 Edge、生产导出 Worker 和 Rust `EditedPlyView`，通过仅限 loopback 的测试二进制适配器连接，而非 Tauri IPC。测试使用 Ballintoy 的 4,096 点夹具，应用删除位图、球裁切及位置/旋转/缩放，最终保留 3,510 点、SH3。夹具取小样不属于应用导出的减点行为。测试参考 PLY 仅在测试编码完成后生成，用于独立比较；生产路径没有此文件。

| 格式 | 实际字节 | 编码耗时 | 回读点数 / SH |
| --- | ---: | ---: | --- |
| 压缩 PLY | 216,905 | 0.180 秒 | 3,510 / 3 |
| SPZ v4 | 78,146 | 0.202 秒 | 3,510 / 3 |
| SOG | 141,694 | 2.823 秒 | 3,510 / 3 |

上述计时为浏览器编码过程，不含整个应用的保存、准备和发布。三种结果均完整解码成功。坐标与编辑后的参考模型进行不依赖排序的最近邻比较：最大误差分别约 0.12747、0.0002062、0.02816 场景单位，均在测试量化容差内。WebGL2 使用相同的三个视角渲染参考 PLY 和三个回读 PLY，像素 RGB MAE 范围约 0.071–0.306 / 255。该样本中 SPZ 小于 SOG，不能将格式排序理解为固定体积排名。

测试输出：`.cache/model-export-1792255946453`；参考模型：`.cache/edited-view-reference-3397c033-b2e5-4755-a2d4-60288d6ebbfd.ply`；渲染结果：`.cache/model-render-1792256054864`。

Ballintoy 全量模型：209,467 点、SH3、源 PLY 49,435,829 字节。本轮同样应用编辑后保留 179,486 点、SH3；原生虚拟视图直接生成的压缩 PLY 为 11,000,982 字节，浏览器编码约 2.268 秒。独立回读后数量、SH 一致，坐标最大误差约 0.62806，低于该模型量化容差 3.28878。输出位于 `.cache/model-export-1792256000404`。

此前未编辑基线：全量 Ballintoy 压缩 PLY 为 12,838,247 字节；原拍摄相机三个方向的 WebGL2 RGB MAE 约 0.89–0.92 / 255，已查看真实画面截图。全量原始模型的少量远端点超出 SPZ 当前编码器范围，真实 Worker 已验证返回 `SPZ_POSITION_RANGE`，不会发布损坏结果；本轮小样通过不能替代全量 SPZ 范围验收。

### 500 万点标准 PLY

```powershell
cd src-tauri
cargo test --lib commands::model_export::tests::large_standard_ply_export_smoke -- --ignored --exact --nocapture
```

测试生成 5,000,000 点、SH3、1,180,001,478 字节（约 1.099 GiB）的源夹具。标准 PLY 点数、SH 和文件大小通过校验，压缩估算超过 512 MiB 并被拒绝。新版直接写出用时约 0.53 秒，上次同机 Debug 路径约 260.75 秒；新版不再逐点执行无效变换或 SH 计算，没有中转模型写入。计时不包含源夹具生成，可能受文件系统缓存影响，不代表所有模型、Release 性能或压缩速度。约 2.4 GB 源夹具与最终测试输出在结束后自动清理，未修改实际项目。

该测试没有采集操作系统级峰值内存或磁盘 I/O 计数，不能将耗时下降推导为峰值内存等比例下降。代码及测试验证没有完整中转 PLY；原始文件和最终原子发布临时文件仍需磁盘空间。

## 尚未完成 / 环境限制

- 全量 Ballintoy 的 CPU SOG 编码此前超过 10 分钟测试限时，尚未完成完整输出；本轮仅完成小样 SOG，不能替代全量效果验收。
- 尚未在真实桌面 WebView2 中人工验收所有交互；原生虚拟视图和浏览器适配器验证不能等同于完整 Tauri IPC 端到端验证。
- 尚未实测 WebGPU 输出画面、macOS/Linux 的应用运行或安装包。仅验证共享编码逻辑、WebGL2 与三平台资源映射。
- 磁盘故障覆盖了模拟不完整写入/发布失败保护，没有通过填满真实磁盘验收。
- 尚未完成优化前后进程峰值内存、实际磁盘写入量的系统级对比。

## 重跑工具

```powershell
# Rust 编辑后虚拟视图 → 生产 Worker → 完整回读；输入小型真实 PLY
$env:OOOSPLAT_EXPORT_SMOKE_INPUT="<标准PLY路径>"
$env:SMOKE_FORMATS="compressedPly,spz,sog"
cd src-tauri
cargo test --lib commands::model_export::tests::virtual_view_browser_smoke -- --ignored --exact --nocapture
cd ..

# 独立编码测试；小样选项只作用于测试夹具，不经过 Rust 编辑视图
$env:SMOKE_POINTS="4096"
node scripts/smoke-model-export.mjs <标准PLY路径>

# 完整属性和坐标检查，并生成用于预览对比的标准 PLY
node scripts/inspect-model-export.mjs <参考PLY路径> <测试输出目录>

# 相同视角 WebGL2 对比
node scripts/render-model-export.mjs <参考PLY路径> <回读PLY路径> [...]
```

测试编码输出、坐标误差 JSON 和截图位于 `.cache/model-export-*` / `.cache/model-render-*`，不会登记为项目或提交。直接编码完整模型前应移除测试进程环境中的 `SMOKE_POINTS`；`SMOKE_FORMATS` 可限定 `compressedPly,spz,sog` 中的格式。全量原始 Ballintoy 的 SPZ 范围拒绝属于预期失败，脚本会以非零退出码结束。

## 2026-10-21：取消导出内存门槛与全景缓存上限

### 行为和接口

- 前端、后端准备/编码及过滤索引均移除估算内存拒绝；准备响应仅保留 `exportId/fileSize/splatCount/shBands`，删除预算和是否允许压缩字段。不新增替代比例、弹窗或设置。
- 保留单 Worker、串行 1,024,000 字节 IPC、无中转 PLY、修订/源文件检查、完整回读及原子发布；实际索引分配失败、编码异常、SPZ 越界、非法令牌和不完整写入仍会失败。输出分块及异常输出大小保护不变。
- 全景视频抽帧、图片准备及六面缓存统一取消 20 GiB 固定门槛。单图解码 512 MiB、相机/尺寸、整数溢出与 COLMAP 图像 ID 数量检查继续保留；不调整抽帧、训练配方或分辨率。

### 真实编码

Windows，Rust 编辑后虚拟视图 → 生产浏览器 Worker，经本地 loopback 二进制适配器连接，**不是原生桌面 Tauri IPC 人工验收**。离线 WebP WASM 从本地加载。

受控源模型为 400,000 点、SH3；删除、裁切、旋转、平移和缩放后保留 **342,857 点、SH3**。虚拟视图 80,915,729 字节，旧估算至少 **596,488,934 字节（568.86 MiB，不含索引容量）**，超过原 512 MiB 门槛。测试目录 `.cache/model-export-1792545638790`，参考 PLY `.cache/edited-view-reference-932e5454-5792-40ea-807f-6dfc84fab7f5.ply`。

| 格式 | 输出字节 | 编码及回读耗时 | 最大坐标误差 |
| --- | ---: | ---: | ---: |
| 压缩 PLY | 21,012,550 | 3.452 秒 | 0 |
| SPZ v4 | 5,947 | 5.561 秒 | 0.000144455 |
| 单文件 SOG | 13,510 | 9.143 秒 | 0.000116041 |

三种输出均完整解码并验证 342,857 点、SH3 和属性有效性；独立位置解码确认八组受控坐标均保留，误差低于对应量化容差。**此夹具重复属性较多，极小的 SPZ/SOG 文件不代表真实场景压缩率，也不代表已验证复杂场景画面或所有大模型。** 测试源文件通过临时目录自动清理；参考 PLY 仅为测试对比生成，不是生产导出中转文件。

另用既有 Ballintoy 小样重新经 Rust 编辑视图和浏览器编码，保留 3,008 点、SH3。三种格式全部通过独立坐标校验：最大误差分别为 0.104762 / 0.000203619 / 0.0177681，均在各自量化容差内。结果和回读 JSON 位于 `.cache/model-export-1792545734104`。500 万点标准 PLY 手动测试也通过：1,180,001,478 字节，直接写出约 0.52 秒；夹具和测试输出自动清理。

### 自动检查和验证边界

- 前端：477 测试及 6 条诊断接收端测试通过；`npm run build` 通过，仍有既有转换器模块外置、WASM 默认 URL 和大 chunk 警告。
- Rust 1.99.0：416 通过、18 条显式手动测试默认跳过；两个既有 Windows 符号链接权限测试显式排除。上述大模型浏览器、小样浏览器、500 万点标准 PLY 三条手动测试已分别执行通过。
- `cargo fmt --check`、Clippy `--all-targets -- -D warnings`、三平台打包资源映射和许可证校验通过。
- 覆盖超过旧导出门槛的索引/元数据、三种格式仅启动一次、取消/重试/修订冲突/已有输出保护；保留 SPZ 越界拒绝。全景覆盖原先超过 20 GiB 的预计规模、512 MiB 解码边界、数量溢出、六面共中心/Mask 对齐、缓存命中与损坏重建、中途取消、实际文件写入失败后的清理和重试。
- 不为验证实际写出 20 GiB 缓存，不重跑全景 SfM/训练；未做本轮原生桌面人工画面检查或 macOS/Linux 真实编码，未测系统级峰值资源或实际磁盘耗尽。
- 受限账户首次启动 Edge 提前退出（加密/权限及 GPU 子进程错误），不算编码通过；以正常权限、独立临时浏览器配置重跑后通过。测试脚本新增浏览器提前退出检测和有界诊断，避免静默等待到超时。

复现大模型真实编码：

```powershell
cargo +1.99.0 test --manifest-path src-tauri/Cargo.toml --lib --locked --offline large_virtual_view_browser_smoke -- --ignored --nocapture
```

取消门槛不是资源无限保证。压缩可能耗尽内存，全景可能耗尽磁盘；可捕获错误继续沿用失败与文件保护流程，不承诺从操作系统强制终止或 WebView 内存崩溃中恢复。

## 2026-10-21：SPZ 越界确认与导出副本调整

### 当前行为

- 生产 Worker 在 PLY 编码坐标空间逐块读取全部保留点的位置和对数尺寸；Rust 编辑后视图只应用一次删除、裁切、变换与 SH 旋转。没有完整中转 PLY，也未恢复已取消的内存或全景缓存门槛。
- 合法坐标不调整、不询问。越界时，同一导出弹窗进入 `awaitingConfirmation`，使用已确认的中英文说明；停止加载动画和百分比显示，保持编辑锁，初始焦点在“取消”。取消或 Esc 终止 Worker、释放会话，不显示失败，也不撤销已保存的编辑。
- 设完整包围盒中心为 `c`，居中后的最大轴向距离为 `R`，安全边界 `B = (2^23 - 1 - 2) / 4096`。用户确认后使用 `s = min(1, B/R)`（`R = 0` 时为 1），按需应用 `p' = (p - c) × s` 和 `logScale' = logScale + ln(s)`；不改变旋转、颜色、透明度、SH、点数或阶数。
- 非有限位置或尺寸失败；缩小时检查全部调整后对数尺寸在 `[-10, 255/16 - 10]` 内，超限使用已确认的尺寸错误提示，不新增饱和。仅平移保留原编码器的尺寸量化行为。
- 确认绑定 `exportId` 和请求标识，每次重试重新检查、重新确认。等待期间没有输出临时文件；确认及尺寸检查通过后才请求 Rust 启动编码。开始编码和最终发布前复核修订、源文件及编辑后视图的一致性。
- 结果可选携带 `{center, scale}`，成功弹窗显示“坐标调整／居中参考点／缩放比例”及公式，不写项目 schema，不生成 JSON 附件。原模型、已有导出、编辑数据保留；最终文件继续原子发布。

### 真实编码、完整回读与画面检查

Windows 无头 Edge 执行生产 Worker，使用 Ballintoy 派生的 3,510 点、SH3 小样，并在独立测试副本上增加平移或一致缩放。二进制通信使用本地 loopback 测试适配器，**不等同于原生 Tauri IPC 人工验收**。原输入与测试源文件字节均核对不变。

| 情形 | 确认次数 | 导出副本缩放 | 最大位置误差 | 最大对数尺寸误差 | 结果 |
| --- | ---: | ---: | ---: | ---: | --- |
| 原坐标合法 | 0 | 不调整 | 0.0001220703 | 0.03124857 | 3,510 点、SH3 |
| 远处紧凑模型 | 1 | 1，仅平移 | 0 | 0.03124857 | 3,510 点、SH3 |
| 平移且跨度超限 | 1 | 0.17090640032943358 | 0.0001220703 | 0.03124714 | 3,510 点、SH3 |
| 取消确认 | 1 | 不执行 | — | — | 零编码、零输出文件 |
| 确认后尺寸不安全 | 1 | 不执行 | — | — | `SPZ_SCALE_RANGE`，零编码、零输出文件 |

独立回读逐点检查坐标和对数尺寸；参考值为同样调整后的原数据，误差处于当前 SPZ 量化容差内。三种成功情形各渲染三个 WebGL2 视角，参考相机同步应用同一变换，RGB MAE 为 **0.132–0.302 / 255**。已查看参考、仅平移和缩放结果截图，保持共同位置与比例；小样包含稀疏点和漂浮点，不构成完整场景质量或无损保证。

证据：`.cache/spz-adjustment-1792553833599/verification.json`；对应渲染目录记录在该文件中。这里的测试 JSON、回读 PLY 均仅用于验证，不是应用导出附件或中转文件。

生产 Worker 与 Rust `EditedPlyView` 再次回归删除、裁切、变换后的 3,008 点、SH3：压缩 PLY **186,139 字节**、SPZ **67,493 字节**、SOG **131,947 字节**，完整回读与坐标检查通过，均未触发坐标调整。证据：`.cache/model-export-1792554057548`。

同时重跑超过旧 512 MiB 估算门槛的受控夹具：保留 342,857 点、SH3，压缩 PLY、SPZ、SOG 均真实编码并完整回读，未恢复内存拒绝。输出字节分别为 21,012,550／5,947／13,510，证据 `.cache/model-export-1792554124467`。该重复属性夹具用于资源门槛回归，极小的 SPZ/SOG 结果不代表真实场景压缩率。

### 自动检查与未完成项

- 前端 493 条测试及 6 条诊断测试通过；`npm run build` 通过，保留既有模块外置、WASM 默认 URL 和大 chunk 警告。
- Rust 1.99.0：417 通过、18 条显式手动测试默认跳过；两个既有 Windows 符号链接权限测试显式排除。小样原生虚拟视图到浏览器的手动测试另行通过。
- 覆盖坐标边界、单点、极远离群点、多块、非有限数据、尺寸上下界和重复读取；覆盖未经确认禁止编码、取消、迟到事件、重试再次确认、修订冲突和源文件变化。弹窗测试覆盖两种语言、默认取消焦点、Esc、Tab 焦点圈定和结果信息。
- Clippy 使用 1.99.0，`--all-targets --locked --offline -- -D warnings` 通过；该工具链未安装 rustfmt，使用现有默认工具链运行 `cargo fmt --check` 通过。资源映射与许可证检查通过。
- 未完成人工原生桌面端到端、WebGPU、macOS/Linux 验收；未新增训练或修改实际项目。无系统级峰值内存测量，不承诺任意大模型均可成功编码。

复现真实调整与渲染检查（输入小型标准 PLY；所有输出位于隔离 `.cache`）：

```powershell
node scripts/verify-spz-adjustment.mjs <small-standard-real-model.ply>
```

### English summary

SPZ preflight scans every retained Gaussian in PLY coordinate space. Legal models export unchanged. Out-of-range models pause in the existing modal for explicit consent, with Cancel focused by default. Only the export copy is centered and, if needed, uniformly shrunk; logarithmic sizes receive `ln(s)`. Unsafe scaled sizes or nonfinite data fail rather than saturating new values. Cancellation and unsafe scaling were verified to create no output file. Revision/source checks, complete readback and atomic publication remain in place; no intermediate PLY, model JSON sidecar, memory gate or schema change was introduced.

Real production-worker encoding/readback and three-view WebGL2 comparisons passed for a 3,510-point SH3 sample, including translation-only and translation-plus-scaling. These browser-loopback tests are not native Tauri manual acceptance. Native desktop, WebGPU and macOS/Linux remain unverified.
