# 埋点与实际 GPU 快照验证

日期：2026-10-27；分支：`main`。

## 采集口径

- 沿用匿名 `planner_evaluation` 汇总：一次任务运行最多一条，内部 OOM 重试及补拍算法回退不另建汇总；手动重试属于新运行。取消不发送失败统计。
- 补拍 V2 在分析、画面准备、最终采用的重建、训练配置解析和结果发布后更新现有 Tracker。失败保留已确认的数据；结果发布和持久化成功前不记录最终产物。
- Source 为本批新增素材的原始尺寸、图片数或视频总帧数；重建与产物为最终采用的新旧合并结果。全景使用原始 ERP 画面，不把六面数当作素材数。
- 恢复优先读取已有源素材快照；旧记录缺失的信息保持未知，不用合并后的训练图片数填充新增素材数量。
- 最后执行阶段独立于计时器；阶段结束后失败仍保留该阶段，日志、心跳及终态不覆盖它。
- 首次调用 Brush 前记录初始配置；只有进程实际启动才更新最终执行配置。未启动的回退不覆盖上一轮实际配置，旧快照无法证明初始配置时保持空值。

## 本地记录和现有上传字段

- 本地任务注册表 `TaskRun` 增加可选 `brush_executions` 和 `current_brush_execution`，记录每次 Brush 的操作、PID、起止、退出码、配置及设备；旧记录可直接读取，不升级项目 schema。
- 每次尝试从空设备状态开始。实际型号只来自本次 `RuntimeTracker` 解析的 Brush 输出；资源采样按进程隔离、唯一型号匹配，硬件缓存也必须唯一匹配。不能确定显存或驱动版本时留空，厂商名不当作驱动版本。
- 退出、取消或任务结束时冻结执行记录；界面清空监控不会清空历史。迟到操作、其他运行和已退出进程的事件不能改写快照。
- 用户确认上传的诊断报告继续使用版本 1：`environment.actualDevice` 为本次实际型号字符串，对应显存和驱动放入 `gpus`；型号已知时保证存在对应 GPU 条目。没有本次设备证据则为 `null`，不从日志尾部或首选显卡推断。
- 各次执行摘要按原顺序放入现有 `detail`，保留根因及最后尝试，遵守 16 KiB 详情和原有报告总大小限制。完整执行记录留在本地。
- 摘要白名单不含 PID、操作 ID、路径、GPU UUID、总线标识或 GPS；继续使用原有脱敏及诊断预览确认流程。匿名埋点不新增设备型号字段。
- 数据库结构、SQL、服务端校验、上传 DTO、事件名、统计版本及生成参数未修改。服务端仅增加现有协议的回归测试。

## 自动验证

- Rust `--lib`：490 通过、0 失败、26 个需要真实素材、引擎或硬件的测试保留忽略。
- 测试覆盖首次失败与重试配置、同 Profile 降点、旧快照未知初始配置、补拍源数量、发布失败不记产物、最后阶段、设备冻结、重启读取、双显卡／同名多卡、缺失驱动或显存、迟到事件、取消、长摘要大小与顺序。
- 现有 TaskService 录制传输回归覆盖新建／恢复／补拍、图片／视频、成功／失败、每运行去重；既有测试继续验证关闭统计、静默传输失败、诊断确认及重复上传保护。
- 诊断服务严格校验及 JSONB 组装测试：7 项通过，确认实际 GPU 与执行摘要仍可使用原字段保存，无需迁移。
- 前端 `npm test`：600 项通过，另含上述 7 项诊断服务测试；`npm run build` 通过。保留既有 PlayCanvas 外部化、资源 URL 和大包提示。
- `cargo +1.99.0 clippy --all-targets -- -D warnings`、稳定版 `cargo fmt --all -- --check`、`git diff --check` 通过。
- Windows 符号链接用例在普通沙箱中存在权限 1314，已在获准的本机测试权限下通过；未通过修改生成逻辑规避该检查。

运行 Rust 检查时仅临时设置 `TAURI_CONFIG={"bundle":{"resources":[]}}`，避免测试编译复制引擎资源；没有修改保存的配置或生成安装包。

## 未执行的实机验收

未故意耗尽 GPU 显存、未重跑完整训练、未向线上服务发送测试诊断。上述硬件与流程边界通过解析回放、隔离文件及录制传输测试验证，不代表新的真实多 GPU 训练或跨平台验收已经完成。

## English summary

Reshoot V2 now feeds the existing telemetry tracker as facts become available, retaining partial failure data and the last actual stage. Each run still emits at most one anonymous planner summary. Brush executions keep separate local configuration and device snapshots; diagnostics use frozen evidence through the existing `actualDevice`, `gpus`, and bounded `detail` fields. Unknown fields remain null. No database, SQL, wire schema, version, generation parameters, or consent flow changes were made. Native hardware OOM and full training were not exercised.
