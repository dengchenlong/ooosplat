# 默认生成策略与自动重试 / Generation defaults and automatic retry

本说明对应当前源码的新策略，不代表旧安装包或已存在的中断快照已经改变。

## 默认配置

原自动优化的重建、闭环和自动补帧成为默认行为；“自动重试”只授权改变预期方案的重试，默认开启。关闭不会关闭补帧或后端兼容回退。

| 档位 | 基础抽帧 | 默认补帧候选目标 | 普通素材训练 | 全景 ERP／六面 |
|---|---:|---:|---|---|
| 快速 | 6 FPS | 9 FPS | 现有优化配方，最大长边1600 | 2048／512上限，不放大 |
| 均衡 | 8 FPS | 12 FPS | 现有优化配方，最大长边1920 | 3072／768上限，不放大 |
| 精细 | 12 FPS | 15 FPS | 原始分辨率 | 原始 ERP，六面边长 `floor(宽度/4)` |

视频滑轨控制首次抽帧比例，不包含补帧。界面手动档位为0.1%、10%、20%……100%，另保留按源 FPS 换算的精确默认值及恢复的旧比例；百分比居中显示。切换质量档位／恢复默认清除手动覆盖。后端比例范围与首次选帧索引不变，保留至少30帧规则，源帧不足时全部采用。图片始终处理全部选中素材。

策略版本3中，仅手动抽帧改为总量最多1.2倍：首次实际抽取`N`帧、源视频可用`T`帧时，最多新增`min(N / 5 向下取整, T − N)`帧。100%不新增。默认档位仍用表中9／12／15 FPS候选预算。初始注册率低于80%、存在有效初始模型时，按未注册区间优先级从源帧范围按需选取，不再受旧候选池密度限制；不提前枚举或抽取全部源帧。只尝试一轮，保存预算及选择，续跑不重复扩大。

候选范围与允许抽取总量分开记录；候选数量不代表已经抽取或将全部参与训练。全景保存生成阶段已读取的完整真实PTS索引，补帧复用它，不另跑整段探测，也不按平均FPS伪造时间。GUI／CLI／MCP新任务和显式失败重试默认版本3；已有版本1／2任务中断续跑保持原规则，升级重试使用现有归档屏障处理不兼容工作缓存。1.2倍只是帧数预算，不是耗时或成功率承诺。[边界与原生验证](MANUAL_BRIDGE_V3.md)。

精细档不按显存分档：普通30000步，阈值0.0015、选择比例0.15、截止24000；全景45000步，阈值0.001、比例0.30、截止38000；Refine均为200步。策略版本2的普通素材采用[有效像素预算](FINE_SPLAT_BUDGET.md)：`round(0.5 × sqrt(2462476 × 0.063362106 × X_eff / meanTrackLength))`；仅统计已注册训练图片，扣除 Alpha 为零的像素，半透明像素仍计入。全景继续使用最终有效球面 COLMAP 稀疏点数×40，不乘六，不保留原1000万硬上限。旧 PLY 初始化超过预算时仅提高至初始化点数，不丢点。上限不是增密目标；数量溢出和引擎数值范围仍校验。旧版本任务续跑保持原快照及原预算。

原分辨率仅针对训练，不取消 SfM 特征尺寸限制。保留512 MiB单图解码等安全保护；无法处理时失败，不暗中缩小。

所有档位仅在实际图片具有非不透明像素时启用两视图三角化，透明视频抽帧也适用；全不透明的 RGBA 不启用。

## 自动重试与兼容

- 明确 GPU OOM 后，同一运行最多重试一次，上限为当前失败进程最后一次可信点数的 `floor(N×4/5)`。不使用配置上限或历史进程点数猜测。
- 没有计数、无法降低、或新上限低于初始化点数时不重试。分辨率、训练步数、学习率及增密参数不变，从原初始化重新训练，不恢复旧优化器。
- 补拍的全量重建／重新训练回退也受此开关约束，保留明确原因和各类一次的原限制。OOM不触发算法回退；取消、崩溃、DeviceLost、资源限制和文件错误不冒充 OOM。
- 完成详情仅在实际执行了受控重试时显示“已自动重试”；只有勾选或发生 Caspar→Ceres 兼容回退不显示。
- 草稿保存独立重试开关和比例；任务保存策略、抽帧、真实参数和重试消耗。失败任务手动重试采用新策略、新运行记录，归档不兼容工作缓存；中断续跑仍使用原快照。已发布模型和编辑成果不归档或覆盖。
- GUI、MCP `generation_policy` 和 CLI `--auto-retry false` / `--frame-sampling-ratio 0.5` 使用同一策略。未提供新选项的新任务使用默认值；旧接口显式禁止补拍回退时仍尊重该拒绝，旧 `plannerEnabled` 仅解释历史行为。MCP 手动重试使用 `start_task.retry_failed_run_id`，旧运行标识不能重复启动新一轮。

## English

Previous optimization behavior is now standard, independently of the default-on Automatic retry switch. Initial video sampling remains 6/8/12 FPS, with 9/12/15 FPS backfill candidate targets. Manual slider detents are 0.1%, 10%, 20% through 100%, plus the exact FPS-derived default and restored legacy values. The slider controls only initial sampling, preserves the existing 30-frame minimum, and never limits a larger manual base selection to the rescue pool. Images are not subsampled.

Policy v3 changes only manual sampling: for `N` initially selected frames from `T` source frames, add at most `min(floor(N/5), T-N)`. The initial indices remain unchanged. Default sampling retains the 9/12/15 FPS candidate pools. Below 80% initial registration with a valid initial model, one bridge attempt searches unregistered intervals lazily across source indices, independently of the old pool. It does not extract all candidates, and resume cannot renew the frozen budget. Panorama bridges reuse the bounded real PTS index already probed during generation, including VFR and nonzero start times. Candidate availability is not the extraction budget or the number of training views.

New GUI/CLI/MCP tasks and explicit failed-run retries default to v3; interrupted v1/v2 runs retain their saved policy. Incompatible retry caches use the existing durable archive barrier. The 1.2× limit controls frame counts, not time or success probability. Training recipes, Automatic retry consent, and existing reshoot dispatch are unchanged. See [scope and native validation](MANUAL_BRIDGE_V3.md).

Detailed uses source-resolution training, including native ERP and width/4 cubemap faces at 92°. SfM feature limits and the 512 MiB image decode guard remain. Policy version 2 uses the [effective-pixel budget](FINE_SPLAT_BUDGET.md) for ordinary media: `round(0.5 × sqrt(2462476 × 0.063362106 × X_eff / meanTrackLength))`. Only registered training views contribute; zero-alpha pixels are excluded, while positive semi-transparent pixels count. Panoramas retain 40× the final valid spherical sparse point count, without VRAM tiers or the old 10M cap. A larger trusted PLY initialization raises the effective ceiling without discarding points. Saved older runs retain their original policy. This is a ceiling, not a promised final count.

Only verified GPU OOM may retry once at 80% of the failed process's last trustworthy reported count. Missing counts, ineffective reductions, or ceilings below initialization fail without guessing. Other training settings remain unchanged. Reshoot algorithm fallbacks also require this switch; compatibility fallbacks and backfill remain standard. Completed details mark actual controlled retries, not permission alone.

New tasks and manual retries use the new policy; interrupted resumes retain frozen legacy settings. Incompatible working caches are archived with a durable reuse barrier, while source files, published models and edits remain protected. All preset tiers enable two-view triangulation only for actual transparent training images, including video frames.

## 验证记录 / Verification

2026-10-28 手动补帧版本3验证见[独立记录](MANUAL_BRIDGE_V3.md)。下方版本1／2记录保留为历史，不代表重新执行完整训练。

2026-10-27 普通精细档预算更新为策略版本2、系数0.5；全景保持原策略。498项 Rust、600项前端及7项诊断测试、构建、Clippy与格式检查通过，真实90张图片只读核算得到2,220,679上限。[实现与验证详情](FINE_SPLAT_BUDGET.md)。以下记录保留为此前默认策略实现的历史验证，不代表重新完成了训练画质测试。

自动测试与原生硬件验收分开记录。本文不代表原生桌面、高分辨率完整训练或硬件 OOM 已验收；不通过故意耗尽显存来制造成功案例。

2026-10-27 Windows 开发环境：

- 前端：600 项 Vitest、6 项诊断服务测试及生产构建通过。覆盖比例滑轨、精度与重置、缓存元数据重规划、重试开关不重检测、队列、补拍问答、历史配置和真实重试标记。
- Rust：479 项单元／接入测试通过，26 项原生或大模型手动测试保持忽略。符号链接安全夹具在沙箱外的相应权限下验证；不跳过安全检查冒充全部通过。
- Rust 1.99.0 全目标 Clippy（`-D warnings`）、本机 Rust 1.97.0 格式检查和差异空白检查通过；1.99.0 工具链未安装 rustfmt，因此未用该版本执行格式检查。测试进程使用临时 `TAURI_CONFIG={"bundle":{"resources":[]}}` 避免开发客户端占用 DLL；正式资源及打包配置未改动。
- 新增覆盖：三档普通／全景预算、显存无关精细档、原始 ERP 尺寸／大于1024的六面配置、点数溢出和初始化保护、80%点数及单次重试、真实 Brush 报告识别、Alpha 与不透明 RGBA 区分、缓存归档失败的持久化屏障、结果保护和手动重试运行隔离。
- 前端构建仍输出第三方模块／大分块等既有警告；不属于构建失败。原生桌面布局、真实高分辨率训练、硬件显存不足后的完整重试，以及 macOS／Linux 端到端验收未在本次执行，不将单测视作这些验收的替代。
