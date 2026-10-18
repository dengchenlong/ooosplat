# 多格式模型导出验证记录

## 实现范围

本功能在 `main` 实现，未修改版本、项目 schema 或生成参数。全景实现单独保留在 `feat/panorama-video-v1`（`1f3343a`），没有合并到 main。

- 展开菜单直接显示四种格式及说明，不预约导出、不扫描模型、不生成文件；选择格式后才保存编辑并预约对应格式。没有第二层格式选择弹窗。
- Rust 准备可随机访问的编辑后数据视图，不生成完整中转 PLY。无过滤时使用源点数；仅删除时通过位图生成索引；有裁切时扫描位置。索引计入压缩内存预算。
- 虚拟字节按需从原文件读取，仅对需要的点应用一次变换及 SH 旋转，使用预缓存属性偏移和栈上计算缓冲。完全无编辑时直接读取原文件；标准 PLY 只写最终格式的原子发布临时文件。
- 浏览器独立 CPU Worker 按需加载固定版本转换器；单次二进制 IPC 最大 1,024,000 字节，全局串行请求。不通过 JSON/Base64 传模型，不在 React 状态中保存模型数据。
- 支持标准 PLY、压缩 PLY、SPZ v4、单文件 SOG（10 次 SH 迭代）。压缩仍需要整模型编码内存，并非完整流式编码。
- 三种压缩格式遵守相同内存预检；SPZ 另检查当前编码器约 ±2048 的坐标范围，禁止溢出后静默发布。
- 完整解码压缩输出，检查点数、SH 阶数、渲染模型和属性有效性，再发布。透明度量化至 0/1 对应的无穷 logit 是合法值，不误判为损坏。
- 固定 SPZ 绑定在 Vite 构建时适配为静态闭包，不更改 WASM、不修改 node_modules，不开放 JavaScript `unsafe-eval`。WebP WASM 本地打包；测试使用严格 CSP，仅允许本地请求。

## 已执行

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
