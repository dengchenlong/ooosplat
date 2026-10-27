# 补拍 v2 / Reshoot v2

开发与验收记录，2026-10-26。对应 [PR #69](https://github.com/ooolabdev/ooosplat/pull/69)，不代表所有平台已完成发布验收。

## 使用方式

从已完成项目创建补拍草稿。默认继承质量与自动优化，启动前可调整。视频沿用正常生成的抽帧；超过100张补拍图片需回答顺序问题。依次询问原模型情况、普通素材的拍摄设置、必要的图片顺序，不增加方案确认页；全部必要问题回答后才创建任务。全景跳过拍摄设置问题。GUI自动回退随自动优化开关保存；已创建任务沿用原快照，不追溯改写。

| 原模型情况 | 初始方案 |
| --- | --- |
| 整体正常，只想补充细节 | 增量重建＋旧模型初始化、正常配方训练 |
| 整体结构正常，但生成效果不满意 | 增量重建＋重新训练 |
| 明显变形或位置错乱 | 全量重建＋重新训练 |
| 不确定 | 增量重建＋重新训练 |

无法证明旧 PLY 与训练相机坐标一致时，采用重新训练；第一选项说明原模型不可用时重新生成。外部导入模型不用于微调。所有工作在独立派生项目中进行，原素材、原项目、已发布 PLY 与编辑结果不改写。

## 重建和缓存

- 普通补拍允许跨分辨率、宽高比、设备和镜头。选择拍摄设置“相同”、源模型只有一组普通相机且全部实际工作图片尺寸/方向兼容时，用`existing_camera_id`复用原内参；其他情况普通照片逐张独立、视频批内共享。不凭同设备名或同尺寸自动合并。答案与实际相机分组保存到检查点，旧任务不追溯改变。
- 原相机 ID 和已注册图片引用保留，最终稀疏模型的优化后内参仅同步到派生数据库。增量固定旧位姿和内参；全量不加载旧模型。局部/全局 BA 显式 Ceres。
- 小规模枚举新旧图片对；大规模用捆绑 COLMAP 的词袋检索，仅索引已注册旧图，每张新图取30个候选，零新增注册时扩大至60并重试增量一次。新增视频/已排序图片使用窗口10；其他图片穷举；闭环继承档位与自动优化规则。
- ERP 补拍保留原始球面 SfM；各批可有不同尺寸。仅注册成功的画面生成92°六面，面上限512/768/1024且不放大。RGB和Alpha同投影；点云保留3D初始化点，移除失效ERP观测与轨迹。
- 六面映射记录批次、原文件名、身份/方向、时间或自然顺序、ERP和派生相机；图片不伪造PTS。自动定位取最早有效来源的front面。
- 来源身份、问题答案、初始/实际方案、回退次数、实际训练参数和初始化哈希保存到可选检查点，不升级项目schema。旧任务缺少recipe时继续旧路径。恢复未完成训练是重新从该次初始化开始，不恢复优化器。
- 大型处理在阻塞工作线程执行；逐图/逐面转换和完整PLY校验支持取消，不并行驻留所有源图。

## 训练参数

新建任务保存`normalParametersVersion: 1`，直接使用对应普通/全景正常生成的参数解析器和任务配置快照。质量档位、自动优化、显存及实际尺寸决定完整步数、分辨率、Refine、增密、学习率和Splat预算；不再使用补拍倍率。视频调用原有`prepare_frames()`，自动优化开启时也使用正常工作分辨率规划。

第一选项仍可从可信的未编辑PLY初始化，并新建优化器；正常预算解析使用实际旧PLY点数，不再用稀疏点数代替。普通配方未指定上限时不另加上限，全景仍受1000万硬上限保护。旧图与新图共同训练，不冻结旧Gaussian，不保证仅改变新增区域。完整配方可能比历史微调耗时更长。

以下为**历史配方**，只供已有任务恢复和旧实测记录解读；缺少新策略字段的任务保留它，新任务不再采用：

| 参数 | 快速 | 均衡 | 精细 |
| --- | --- | --- | --- |
| 基础训练步数 | 3000 | 5000 | 8000 |
| 实际步数 | `min(完整配方步数, max(基础步数, 10×训练视图数))` | 同左 | 同左 |
| Refine | 200 | 200 | 200 |
| 梯度增密开始/截止 | 约15%/80%，对齐200步 | 同左 | 同左 |
| 增密阈值/选择比例 | 完整配方×1.5 / ×0.5 | 同左 | 同左 |

位置学习率初始/最终为Brush默认值×0.25；颜色、透明度、尺寸、旋转×0.5；位置噪声×0.1。保持原SH阶数和初始化点数，预算不得低于初始化点数；全景硬上限仍为1000万。完整重训不使用微调倍率。不冻结旧Gaussian，也不保证只改变补拍区域。

新建GUI补拍只有开启自动优化才自动允许零新增注册/旧坐标不兼容时全量重建一次，或明确数值发散/正常结束输出非法时冷启动一次；MCP仍使用显式`allowFallback`参数，旧任务保留已保存授权。OOM、资源限制、进程崩溃、磁盘/文件错误、素材变化和取消不触发算法回退。低但非零注册只警告。原有有界OOM保护独立保留；不丢弃初始化点避开显存错误。各次训练保留独立尝试目录，恢复不重置回退次数。

## MCP

`inspect_reshoot_input` 只读分析已登记源项目及授权输入目录，返回源/输入身份。`create_generation_task` 可同时传入 `source_project_id` 和显式 `reshoot_options`（condition、allowFallback、sourceIdentity、inputIdentity、projection、可选imageMatchingStrategy及cameraRelation），然后照常`start_task`。cameraRelation为same/different/unsure，省略时独立处理；已有任务恢复保持原相机及参数快照。创建和执行前均重新校验身份。

## 日志与完成状态

补拍单独增加原项目文件、画面准备/整理、蒙版检查、相机决策、注册、初始化和缓存日志；同类计数最多每秒一次，首尾必发。应用侧15秒无结果时提示，后续最多每30秒一次，不冒充引擎输出或推进百分比。取消/切换操作后停止提示。普通生成日志不变。

进入最终校验/发布阶段清理训练监控；只有文件校验、原子发布及项目结果持久化成功后才记录“已发布N个Splat”并完成任务。8000/8000等训练步数不代表结果已发布。完成详情不显示旧的“等待结果校验”监控，不改写历史记录或补造日志。

## 实测证据与限制

Windows x64，本机RTX 3060 Ti 8GB，捆绑COLMAP 4.2.1和现有OOOBrush；使用Ballintoy真实ERP图片与已有球面模型/原始生成PLY。

以下为旧版缩短训练配方的历史记录；正常完整配方的本次验证见下一节。

- 原项目56张已注册画面（1920×960），新增8张真实ERP图片（准备后2048×1024）。相机组不同，未将新图缩放至旧相机尺寸。
- 8/8新增画面注册成功，共64/64。旧模型209,467点；微调3000步后269,068点，输出63,501,665字节，总耗时225,753ms。原`final.ply`前后SHA-256一致。
- 首次隔离目录：`.cache/reshoot-v2-native/e2f76af79f0c4232ba5f4140fce0bea9/`。独立尝试目录和阻塞线程调整后复验：`.cache/reshoot-v2-native/9637c9a81e88446bb9406419451fba09/`，64/64注册、270,141点、63,754,893字节、229,207ms；源PLY哈希仍一致。两次均保存`events.json`、`result.json`、派生项目/引擎日志，不提交测试缓存。
- 使用隔离复制数据库真实执行`vocab_tree_retriever`：5张旧图、2张查询图、每张3候选，得到6对且全部属于指定旧图集合；约5秒。证明当前捆绑引擎支持分别索引旧图与查询新图，不以全库检索冒充30/60张旧候选。
- 自动检查：前端539项及诊断服务6项通过，前端构建通过；Rust 438项通过、20项手动/素材测试默认忽略。两个现有符号链接权限测试在本机返回Windows 1314，未修改其断言，单独过滤后完成其余Rust测试。Clippy `--all-targets -- -D warnings`通过。
- 交互精简复验：移除重复拍摄提醒和方案确认页，处理说明合并至原模型情况选项；GUI自动回退随启动时的自动优化保存。前端541项、诊断服务6项和构建通过；覆盖无默认选择、直接启动、图片顺序前后导航、取消、重复点击及开关参数。未重新执行训练；选项新增文案已获用户确认，桌面人工验收仍未完成。
- 第二次原生运行的资源采样：COLMAP进程观测内存最大160.5MiB，Brush为674.4MiB；显卡整卡已用显存观测最大3213MiB（含桌面和其他进程，不能当作本任务独占显存或真实峰值）。六面训练目录173,119,985字节。采样不包括应用进程转换图片的内存峰值。
- 测试直接调用生产执行核心，未登记到全局项目目录；不等同于已完成Tauri/桌面点击端到端验收。素材来自同一拍摄序列，不是新拍摄或真实跨设备/镜头验证。
- 未完成：真实跨设备普通图片/视频对照、不同相机镜头组合、连续多次原生补拍、全量及授权回退真实失败场景、桌面问答/队列操作、取消/磁盘故障原生验收、全景接缝和画质人工对照、macOS/Linux原生验收。
- 自动测试使用小型数据及隔离子进程覆盖参数、映射、状态和错误边界；不能据此声称所有硬件兼容或微调一定提升画质。训练过程尚未记录完整峰值显存/内存对照；资源采样不是峰值测量。

## 本次修复验证（2026-10-26）

- Windows x64 / RTX3060 Ti 8GiB，真实Ballintoy：56张旧画面＋8张新增ERP，64/64注册；从209,467点旧PLY初始化，采用正常快速档12,000步、512单面、100万上限、0.0018梯度阈值、0.20选择比例、10,000步梯度增密截止。未使用补拍学习率覆盖或缩短步数，未触发OOM重试。
- 结果578,827点、136,604,789字节，总任务358,226ms（训练进程约173s）。原PLY SHA-256前后一致；事件记录含真实准备计数、8/8新增注册以及“已发布578827个Splat”，随后才完成。目录：`.cache/reshoot-v2-native/d2aedcf973454d7caea88688b3ed1e27/`，保留events.json、result.json和派生项目，不提交缓存。
- 实际捆绑COLMAP、新隔离数据库、真实原PNG执行CPU特征提取：原图与确认相同的新图共享一个camera_id，独立新图生成第二个camera_id；原PNG SHA-256不变。测试`native_reshoot_features_confirm_reused_and_independent_camera_ids`实跑通过，不代表跨设备重建效果已验收。
- 自动测试区分正常配方与旧快照，覆盖显存边界、点数保护、无额外上限、相机兼容、日志限频/迟到隔离/取消、非法候选及写入失败不发布。原生桌面显示、真实跨设备/镜头、普通视频补拍完整链路和新配方画质对照仍未验收；上述核心执行测试不能替代它们。
- 前端559项及诊断服务6项通过，构建、Rust格式检查和Clippy通过。Rust全量446项通过、23项默认忽略；2项既有符号链接测试因Windows权限1314失败，未放宽断言。上述两个原生素材测试已单独显式运行通过，不属于mock验证。

## English summary

New reshoots use the normal full ordinary/panorama training recipe, including when initialized from the unedited old PLY. There are no shortened warm-start steps or reduced learning/densification multipliers. Existing tasks keep their saved recipe. Ordinary inputs additionally ask whether capture settings match; original intrinsics are reused only after explicit confirmation and compatible single-camera pixel checks. Otherwise the current independent-camera strategy remains. ERP batches stay independent. Reshoot-only preparation/cache logs are throttled and cancellable; successful publishing logs the verified PLY count, and completed tasks no longer show a stale training monitor.

Reshoot v2 preserves the source project and writes a derived project. Ordinary inputs may differ in size/aspect/device/lens; full monoscopic ERP inputs may differ across batches. Ordinary and ERP projections cannot be mixed. Conservative independent photo cameras avoid falsely sharing intrinsics. Incremental reconstruction fixes old poses/intrinsics; full reconstruction does not load the old sparse model. Both use explicit Ceres BA.

The four model-condition answers choose incremental+warm, incremental+cold, full+cold, and incremental+cold respectively, with plain-language explanations in the choices and no separate plan-confirmation page. For new GUI reshoots, Auto Optimize also enables slower algorithm fallback; switching it off disables that fallback. MCP retains its explicit `allowFallback` parameter, and existing tasks keep saved decisions. Fallbacks remain bounded across resumes; resource/process/storage failures never trigger algorithm fallback. Warm-start uses the unedited generated PLY and a new optimizer, not optimizer-state continuation. Old and new views both train.

Earlier Ballintoy tests used the historical 3000-step warm recipe. The current normal-recipe test registered all eight new ERP images (64 total), initialized from 209,467 splats, and published 578,827 splats after 12,000 steps in 358,226 ms. The source PLY hash was unchanged. A separate real COLMAP extraction test verified shared versus independent camera IDs and unchanged source PNG bytes. These are Windows core tests, not desktop or cross-device acceptance. Desktop interaction, ordinary-video reshoot end-to-end, macOS/Linux, visual quality comparisons, and peak resource measurements remain unverified. See the Chinese sections above for evidence paths and automatic-test results.
