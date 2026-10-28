# 图片 GPS / Image GPS

开发及验证记录，2026-10-26。仅适用于新创建的图片生成与图片补拍；已有任务缺少 GPS 快照时继续原流程。不升级项目 schema，不增加弹窗或设置。

## 使用和范围 / Usage and scope

- JPG／JPEG／PNG 中的标准 EXIF、XMP GPS 会自动识别。输入框只显示已确认的提示，不显示经纬度或数量。最多均匀抽样16张、最多四个读线程；不解码像素、生成预览或计算整图哈希，切换质量不重新检测。
- 生成时读取实际采用图片的 GPS，通过工作图片映射关联，不能靠重编码副本是否保留 EXIF 判断。抽样未发现 GPS 不阻止生成阶段使用它。
- GPS 仅补充最多50个水平近邻候选，不删除原有顺序、穷举、闭环或补拍新旧匹配。穷举任务通常已经包含这些图片对，因此不保证提速。
- 仅有经纬度或三维几何不可靠时，仅使用候选匹配，不声称已定向或确定尺度。至少5张有高度的、位置可区分且非近共线的图片，并且水平跨度达到 `max(5m, 3×水平误差中位数)`，才尝试三维位置软约束。
- 默认水平／垂直标准差为10m／20m；有有效水平定位误差时采用该值（0.1–1000m）。不将 DOP 当作米。显式非 WGS84、来源冲突、损坏及元数据／解压预算超过64KiB时跳过，不影响正常图片识别。
- 当前只在 Windows x64 启用原生 GPS 路径；macOS／Linux 尚未原生验收，自动采用原有视觉流程。全景图片与图片补拍已接入，但其真实素材验收范围见下表。

Embedded EXIF/XMP GPS is optional and automatic. Fast inspection samples at most 16 images without pixel decoding, previews or full-image hashes. The generation pass reads all selected image sources. GPS supplements, rather than replaces, the selected matching strategy. Horizontal-only observations can suggest pairs but cannot establish 3D orientation/scale. Native GPS processing is currently gated to Windows x64; other platforms retain the visual-only workflow.

## 重建、坐标及保护 / Reconstruction and protection

原始视觉重建先保留为可用回退。在独立数据库中按实际 image/camera ID 写入 ENU 位置与协方差，再运行原生 `spatial_matcher`。三维条件满足时调用 `pose_prior_mapper`、鲁棒位置损失、局部与全局 Ceres；不改变原相机模型、主点保护或 Brush 配方。

通过 `model_aligner` 对齐本地 ENU；至少5个有效内点、内点占全部有高度记录的80%以上，归一化位置残差不超过3σ，并再次检查内点几何。原生 Sim3 的序列化格式、输出相机中心、相机和三维点一同验证。转入 PLY 空间后，现有预览映射使未编辑结果 **X向东、Y向上、−Z向北**，长度单位为估计的米。不是测绘精度，也不保证统一高程基准；已有展示变换及 SPZ 导出缩放仍可改变轴向、位置和单位。

失败只在信息／接口不足、退化或验证拒绝时回退一次；取消、文件／磁盘错误、异常退出和资源错误保留真实失败，不冒充 GPS 不足。正常退出但注册数量减少的 GPS 结果不采用。不完整的尝试不会登记为完成。

图片补拍保留原项目只读。增量结果必须保持旧相机和旧位姿；只允许消除共同的全局相似变换后重新严格校验，不容许独立移动旧相机。使用旧 PLY 时，仅转换内部初始化副本的位置、朝向、对数尺寸和 SH，不降点／降阶，不应用预览裁切或重复展示变换。继承的可信地理原点保存在本地；新 GPS 被拒绝也不破坏已固定的旧坐标。

状态及清单保存在 `state.json` 的可选 `gps` 和 `work/gps/manifest.json`；原始坐标不返回前端，也不加入统计／诊断上传。缓存键覆盖来源身份、GPS、策略、视觉模型及地理参考，输出含校验身份；变化时禁止复用依赖旧坐标的训练检查点。图片、Mask、原视觉重建、已发布模型和编辑数据不被 GPS 尝试覆盖。旧任务不追溯改写。

The visual reconstruction remains a separate fallback. Native pose priors use robust Ceres optimization; validated results are aligned to local ENU and mapped into the existing PLY/preview convention. All cameras and sparse points share the transform. Reshoot Gaussian initialization uses the same similarity, including SH rotation, without editing the source. Local manifests are not telemetry or diagnostic attachments. A geographic result is an estimate, not a surveying guarantee.

## 验证证据 / Validation

真实公开样例：[Brighton Beach](https://github.com/pierotofy/drone_dataset_brighton_beach/tree/9506a9bf09c9678b7d0b80b014af5b606c8a7255)，18张带 EXIF GPS 的照片，BSD-2-Clause，Piero Toffanin，2017。使用固定提交；源码 ZIP 为69,435,204字节（约66.2MiB）。样例不随安装包分发。

- 固定 ZIP SHA-256：`1491a6c196bff1890850628d42fed2576443af3344a60c5f7eca14180ff10220`。
- COLMAP：应用附带4.2.1，原生位置先验、匹配、Ceres、对齐和轴转换实际执行；Brush 使用捆绑运行库、RTX3060Ti、Vulkan。
- 隔离任务：`.cache/gps-v1/public-projects/20261026-204112_images`；18张全部注册，快速档8000步，输出 **104,967 Splat**。GPS对齐日志保存在 `logs/gps.log`，统计在 `gps-validation.json`。全部原图前后 SHA-256 一致。
- 本次快速抽样约 **1ms**（原图哈希已读，属于热缓存），完整流程约 **110.6s**。采样到的子进程峰值常驻内存约 **375.9MiB**；整卡采样显存峰值 **2723MiB**（包含其他程序，不是精确的进程显存或瞬时峰值）。对齐18张参考图片，位置残差均值约 **0.387m**、中位数约 **0.349m**；这是对参考GPS的差异，不是真值精度。不是跨磁盘／硬件性能承诺。第一次解析器格式不兼容的尝试确实安全回退并完成普通训练；修复后再次通过原生 GPS 闭环。
- `scripts/smoke-colmap-gps.mjs` 在真实引擎上验证普通和 ERP 模型、实际数据库 ID、六个位置先验、Ceres 和10倍尺度恢复；这是合成几何能力测试，**不等于真实全景 GPS 效果验收**。
- 普通图片补拍集成实测：以上公开照片再取6张作为独立补拍批次（重复照片，不代表新增场景覆盖），使用旧 PLY 初始化＋正常快速档8000步，约211.9s完成，输出500,000 Splat。GPS为`complete`，初始化记录包含统一坐标变换；原项目模型、数据库、元数据及检查点哈希不变。隔离证据：`.cache/gps-v1/reshoot-projects/214b331770fe47d69ac58662406d35a2/projects/20261026-204828_input`。这验证传递和发布，不证明跨设备补拍画质。首轮测试误选了冷启动配方，训练完成但初始化断言失败；改正测试为旧模型初始化后重新通过，未将首轮误报为微调通过。
- 仅经纬度分支：将上述真实照片对应的本地测试记录去掉高度（不改原图），实际运行原生匹配与 Mapper；状态为`matching`，不生成新的地理定向标记或 PLY 变换。实测缓存命中不重算，损坏输出后重建缓存，源数据库哈希不变；约6.4s完成两次尝试。证据：`.cache/gps-v1/horizontal/6f036a33ef8d46aca3f1b3cd8c34cb5b`。
- 自动测试覆盖 JPEG EXIF、PNG eXIf／压缩及未压缩 XMP、命名空间、DMS／分数／南西半球／跨日期线、合法零值、缺失高度、冲突、坏 CRC、解压预算、抽样上限及生成全量读取、取消、真实 ID／协方差、相似变换、旧 PLY SH／数量／尺寸及源字节不变、缓存和旧任务兼容。
- 前端验证包括两种语言、历史和视频不冒充使用 GPS、素材切换的迟到响应隔离、质量切换去重及可选检测失败不阻塞生成。采用 React 检查表将检测独立于规划请求，避免 GPS 变成启动依赖。

自动检查：前端591项＋诊断6项通过；构建、Rust格式检查、Clippy（all-targets，warnings为错误）、许可证检查通过；引擎脚本75项通过、8项按环境跳过。Rust全量首次为466项通过、2项因Windows符号链接特权1314无法创建测试夹具、其余忽略；明确过滤这两项后466项通过、26项忽略。没有把缺少权限的测试报为通过，也没有改动这两项测试或安全限制。

复现（PowerShell，临时目录位于项目缓存，不改写样例）：

```powershell
$env:TEMP = 'E:\Project\OOOSplat\.cache\gps-v1'
$env:TMP = $env:TEMP
node --experimental-sqlite scripts/smoke-colmap-gps.mjs engines/colmap/bin/colmap.exe
cargo +1.99.0 test --manifest-path src-tauri/Cargo.toml --offline --lib gps
cargo +1.99.0 test --manifest-path src-tauri/Cargo.toml --offline --lib native_public_gps_generation -- --ignored --nocapture
cargo +1.99.0 test --manifest-path src-tauri/Cargo.toml --offline --lib native_public_gps_matching_and_cache -- --ignored --nocapture
cargo +1.99.0 test --manifest-path src-tauri/Cargo.toml --offline --lib native_public_gps_reshoot -- --ignored --nocapture
```

### 未完成验收 / Not yet accepted

真实带 GPS 的全景图片、跨设备／镜头补拍、视频来源项目加图片 GPS 补拍、漂移数据集的有／无GPS系统对照，以及 macOS／Linux 原生能力和桌面预览人工检查仍待验收。没有这些素材或平台实测，不能声称全景接缝、跨设备效果、地理精度或所有平台已验证；数学单测及合成 ERP 不替代这些检查。

### CI 兼容修复 / CI compatibility fix (2026-10-26)

提交 `6519351` 的 Windows、Ubuntu、macOS Actions 均已通过前端与 Rust 测试，但在 Rust 1.99.1 Clippy 的 `chunks_exact_to_as_chunks` 检查处失败。本地此前的 Clippy 1.97 未检出这条检查；使用本地1.99.0已复现相同错误。

将 Gaussian 初始化回归测试中的 `chunks_exact(4)` 改为 `as_chunks::<4>().0.iter()`，直接读取固定长度数组。只改变测试代码，不改训练／导出行为、不忽略警告、不降低 CI 检查。修复后本地1.99.0全目标 Clippy（`-D warnings`）、格式检查及13项 GPS 单测通过，3项原生手动测试未重跑。本机开发客户端占用引擎DLL，单测仅通过进程级 `TAURI_CONFIG` 临时跳过资源复制；正式打包配置未变。三平台完整远端构建仍需推送后验证。
