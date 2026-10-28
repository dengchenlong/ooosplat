# 手动抽帧桥接预算 v3 / Manual-sampling bridge budget v3

## 行为与兼容

2026-10-28，当前 main 工作区。仅当内部生成策略版本至少为3且设置了手动抽帧比例时启用。应用版本仍为0.7.1，项目 schema、上传协议和界面文案不变。

```text
N = 实际首次抽取数量（含最少30帧规则）
T = 源视频可用帧数
新增上限 = min(floor(N / 5), T - N)
总量上限 = N + 新增上限
```

未设置手动比例时，快速／均衡／精细仍用9／12／15 FPS候选池。首次选帧索引、数量、训练参数和自动重试授权不改变。初始注册率低于80%、有有效初始重建且存在可补区间时，仅进行一轮定向补帧。内部断点先于边缘，较大间隔优先，中点距离相同时选择较小源帧索引。

新增可选`bridgeSourceFrames`保存可搜索的源帧范围；`rescueMaxFrames`仍表示补帧后允许的总量。搜索范围不等于已抽取数量，不提前建立整个源视频的候选数组。现有候选统计可表达完整可选来源，耗时预估仍基于实际计划抽帧量。全景保留生成阶段已有的真实PTS读取及100万帧限制，保存完整本地时间索引供后续补帧使用，不新增整段探测或像素解码。

版本1／2中断任务保留旧候选计划。新任务和显式失败重试采用版本3；旧手动策略升级时归档不兼容工作缓存，并先保存禁止复用的状态。原素材、已发布模型和编辑结果不归档或覆盖。取消后不将部分选帧计划当作成功，不重获预算。

本轮不增加补拍原本没有的Bridge调用，不修复另行发现的全景精细档预算提前冻结问题，不更改Splat上限或训练配方。1.2倍是帧数上限，不是耗时或成功率承诺。

## 自动测试

- Rust库测试511项通过，28项原生／资源测试默认忽略；其中本次新增的原生抽帧测试另行显式执行通过。Windows符号链接夹具在具备权限的执行环境完成，没有跳过安全测试。
- 前端635项、诊断协议7项通过；生产构建、Rust 1.99.0全目标Clippy（`-D warnings`）、稳定版rustfmt及补丁格式检查通过。构建仍有现有第三方模块、WASM路径和大分块警告。
- 覆盖三档默认计划完全一致、手动0.1%／20%／50%／90%／100%及旧非标准比例、整数取整、短视频、全帧输入、无注册、80%边界、无可补区间、重复排除和确定性。
- 1,800帧案例：20%为360＋最多72；50%为900＋最多180；90%为1,620＋最多180；100%不新增。版本1／2首轮索引与新版相同，训练及分辨率解析结果保持一致。
- 覆盖旧候选池外选帧、超大虚拟源范围不物化、冻结预算与部分提取快照、取消、归档失败持久化屏障及恢复、源帧范围与实际抽帧统计分离。
- 低FPS首次选帧直接计算旧候选池对应位置，不再先物化旧池；1,200万帧／6 FPS／0.1%案例仅保存1.2万张首次选择及其紧凑候选记录，允许总量1.44万，未建立1,200万项数组。
- 全景覆盖VFR、非零起始PTS、缺失／损坏时间索引及真实新增帧时间，不使用平均FPS替代。

## Windows 原生小视频验证

显式运行`native_manual_bridge_v3_extraction_and_rollback`。使用捆绑FFmpeg／FFprobe创建独立的128×64短MOV，不使用或改写用户测试素材。证据保留在：

`.cache/manual-bridge-v3/90a8d0d5-604e-446c-b78a-d626264bc3b9/`

| 用例 | 首次实际帧数 | 实际新增 | 合计／总预算 |
|---|---:|---:|---:|
| 普通RGB路径 | 60 | 12 | 72／72 |
| 普通Alpha路径 | 60 | 12 | 72／72 |
| 全景路径、VFR及非零起始PTS | 36 | 7 | 43／43 |
| 全景Alpha路径、VFR及非零起始PTS | 36 | 7 | 43／43 |

四个用例均核对新增帧索引、原帧字节不变、RGB／Alpha／Mask校验、已取消提取不损坏原帧，以及回退移除新增帧后可恢复原计数。源MOV前后逐字节一致。全景用例直接核对新增帧与已保存的真实PTS，不再次调用FFprobe。

注册集合使用测试夹具，仅验证定向选帧和真实引擎抽帧；合成图案不是全景重建质量样例。本次没有运行COLMAP重建或Brush训练，不宣称注册率或画质提升；原生桌面、macOS／Linux及真实场景补帧效果尚未验收。

## English

Policy v3 applies a total 1.2× ceiling only to manual video sampling: add at most `min(floor(N/5), T-N)` using the actual initial count. Default candidate pools, initial indices, training recipes and retry authorization remain unchanged. A single eligible bridge searches source-index gaps lazily, excluding existing frames. Candidate availability and allowed extraction totals are separate. Panorama bridges reuse the bounded real PTS index read during generation, without another full probe.

Saved v1/v2 runs retain their old behavior; new tasks and explicit failed-run retries default to v3. Upgrades archive incompatible work behind the existing durable reuse barrier, preserving source media, published models and edits. No application-version, project-schema, cloud-protocol or UI-copy change is included. The separately identified panorama Detailed budget dispatch issue is not fixed here.

511 Rust tests, 635 frontend tests, 7 diagnostic tests, build, Clippy and format checks passed. The additional native test passed all four isolated RGB/Alpha and panorama-timing cases listed above, including cancellation, rollback and byte preservation. Low-FPS initial indices are computed without constructing the legacy full-source pool; a 12-million-frame fixture retains only its 12,000 initial choices and compact candidate records. Registration was a fixture: no SfM or training was run. Desktop interaction, other platforms, and real-scene registration/quality gains remain unverified. A frame budget is not a runtime or success guarantee.
