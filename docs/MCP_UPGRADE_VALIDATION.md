# MCP 生成与模型导出升级验证

日期：2026-10-29；分支：`main`；应用版本保持 `0.7.1`。

## 实现范围

- 保留原有八个工具，新增素材预检、指定运行恢复及四个模型导出工具，共14个。预检复用 GUI 轻量分析、身份缓存及规划；不另建 Runner。
- 缺少用户答案时返回 `CONFIRMATION_REQUIRED` 和现有中英文问题，不创建任务。素材、配置或补拍源变化使旧预检失效；重复请求标识可找回原任务，不重复生成。
- 补拍、投影、图片顺序及自动重试映射到原有参数；策略 v3、Bridge 手动抽帧预算及训练配方未改动。全景精细档现有入口差异未在此修复。
- 恢复只接受匹配运行标识的取消／中断任务，要求可读检查点及实际工作图片；Runner 仍逐阶段校验可复用数据。失败任务走显式手动重试，完成任务不能重启。
- 状态增加实际投影、匹配、抽帧／补帧、GPS摘要及每次运行最多8项 Brush 执行证据。未知值留空；不返回 GPS 原点、逐图清单或硬件唯一标识。
- 导出执行器在应用级初始化，不依赖预览；复用现有虚拟编辑视图及单 Worker。只采用已保存编辑，目标编辑和 GUI/MCP 导出互斥。`model_export.busy` 也反映 GUI 导出占用。
- 输出固定为项目 `exports/mcp-UUID.*`，通过同文件系统硬链接原子创建且不覆盖竞争文件；重定向目录拒绝。取消、失败或未获 SPZ 批准不发布结果；重载／重启后未完成会话中断，不继承批准。
- 本地导出注册表使用独立 `mcp-exports.json`；未修改项目 schema、云端字段或上传协议。没有加入模型编辑、任意文件操作、PSNR、HTML或视频导出工具。

## 自动验证

| 检查 | 结果 |
| --- | --- |
| `npm test` | 71个文件、641项前端测试通过；7项现有诊断协议测试通过 |
| `npm run build` | TypeScript、HTML查看器和前端生产构建通过；保留现有 PlayCanvas 外部化、WASM URL及大包提示 |
| Rust `cargo +1.99.0 test --offline --lib` | 522通过、0失败、28项现有条件性忽略 |
| `cargo +1.99.0 clippy --offline --all-targets -- -D warnings` | 通过 |
| 稳定工具链 `cargo fmt -- --check` | 通过 |
| `git diff --check` | 通过 |

新增回归覆盖：无默认答案、30／100／101／500／501张边界、疑似投影与不支持ERP、补拍问题顺序、重试授权、素材／配置变化、重复请求、零重复图片头检测、启动前身份检查、恢复门槛、执行摘要裁剪与隐私、导出预约复用、未保存编辑拒绝、实例与确认隔离、迟到进度、取消和监听清理、重启中断、无覆盖发布及目录跳转拒绝。原有视频元数据、旋转MOV、Bridge、全景转换与训练参数测试继续通过。

官方 `rmcp` 客户端测试使用真实本机 HTTP 传输和工具发现，调用预检、创建、启动、查询、日志及取消，并覆盖断线重连、运行锁和 Host／Origin／目录限制。生成执行器受控，不运行 GPU 训练；新导出工具的原生桌面端到端链路未以该测试冒充验收。

### Windows 测试环境说明

- 编译时仅临时设置 `TAURI_CONFIG={"bundle":{"resources":[]}}`，避免复制引擎资源；未改保存的打包配置。
- 当前环境的 Rust 测试 EXE 没有 Common Controls v6 激活信息，最初启动返回 `0xC0000139`。仅在忽略的 `target/debug/deps/` 测试产物旁放置 v6 外部 manifest 后运行成功；未修改应用源码、引擎或安装包。
- 普通权限下临时符号链接用例返回 Windows 1314；经授权的本机测试环境通过。沙箱内 Edge GPU子进程启动失败，经授权运行无头浏览器后编码通过。未绕开或删除这些安全用例。
- 实施前检查磁盘空间，E盘约13.5 GiB可用；未清理现有素材、项目、缓存或工作树。

## 真实编码与回读

使用现有缓存真实模型中截取的2048点、SH3小样本，只在新的 `.cache/` 目录生成结果，不重训、不覆盖原项目。

| 格式 | 编码时间 | 文件大小 | 回读 |
| --- | ---: | ---: | --- |
| 压缩PLY | 0.172秒 | 127,290字节 | 2048点、SH3，坐标误差在既有格式容差内 |
| SPZ | 0.124秒 | 45,866字节 | 2048点、SH3，最大坐标误差约0.000206 |
| SOG | 1.222秒 | 120,831字节 | 2048点、SH3，坐标误差在既有格式容差内 |

标准PLY由真实 Rust 虚拟视图写出／回读及发布测试验证，包括变换、裁切、删除只应用一次、源文件不变及旧导出保护；它不经过压缩 Worker。

编码证据：`.cache/model-export-1793275431489/{results,roundtrip}.json`。另运行现有 `verify-spz-adjustment.mjs`：合法坐标不调整、仅居中、居中并缩小（约0.248272倍）均保留2048点及SH3；取消和不安全缩放均零编码输出文件。逐点坐标／尺寸量化误差通过，WebGL2记录了三视角对比，不宣称压缩无损。样本哈希前后一致。

SPZ证据：`.cache/spz-adjustment-1793276026665/verification.json`。测试脚本在 Node 中运行仅用于验证；应用没有新增 Node 运行时或第二套编码器。

## 前端检查与未完成验收

依据 React 检查表复核：执行器每次应用加载只初始化一次、订阅先于就绪注册、退出清理全局监听；预览只订阅目标导出的布尔占用状态，编码进度不驱动预览重渲染。未新增按钮、弹窗文案或设置。

尚未执行：原生桌面中 Agent→保存编辑→无预览导出／SPZ询问的完整人工验收，真实普通／全景／补拍的 MCP 全流程 GPU生成，硬件OOM、真实磁盘耗尽、WebGPU导出画面对照，以及 macOS／Linux原生平台验收。模拟生命周期、真实小模型编码和 Windows子进程测试不等同于这些验收。

保留现有未提交内容与测试素材；未提交、推送、改版本号或生成安装包。

## 后续 Windows Actions 启动修复（2026-10-29）

提交 `28dcaf8` 的 [Windows Actions #40894838513](https://github.com/ooolabdev/ooosplat/actions/runs/40894838513) 在 Rust 测试程序启动时返回 `0xC0000139 / STATUS_ENTRYPOINT_NOT_FOUND`，没有实际运行测试；前端、引擎安装和格式检查均已通过。这个问题并非仅限本地环境：Tauri 默认资源通过 `rustc-link-arg-bins` 只链接到程序目标，库测试 EXE 没有 Common Controls v6 清单，而新增应用级调用链使其引入了 `TaskDialogIndirect`。

永久修复在 `src-tauri/build.rs`：仅对 Windows MSVC 目标，由链接器统一内嵌原有 Common Controls v6 依赖，覆盖库测试、程序测试及应用映像；关闭 Tauri 重复的清单资源，保留图标、版本资源及原有无 UAC 片段的行为。macOS、Linux和非MSVC路径不变。参见 [Cargo构建参数](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rustc-link-arg) 和 [MSVC清单内嵌规则](https://learn.microsoft.com/en-us/cpp/build/reference/manifest-create-side-by-side-assembly-manifest?view=msvc-170)。未跳过测试、禁用对话框功能或添加外置清单的CI兜底。

验证：

- 新增 Windows 回归测试，直接从当前 EXE 的资源ID 1读取内嵌 `RT_MANIFEST`，验证 v6依赖；不能因旁边存在外置清单而误通过。
- 把测试 EXE 单独复制到全新的 `.cache/windows-manifest-regression-1379b79ce5b84ca4912a99245b9e7a45/`，确认没有 `.exe.manifest`，仍启动并通过523项测试、28项现有忽略。该独立文件不依赖此前手工放置的清单。
- 完整 `cargo +1.99.0 test --offline`（非仅 `--lib`）通过：523项库测试通过，两个程序测试目标及文档测试均通过。
- 使用 Windows SDK `mt.exe` 直接回读 EXE，确认内嵌 Common Controls v6依赖、没有新增UAC片段。
- `cargo +1.99.0 clippy --offline --all-targets -- -D warnings`、格式检查及 `git diff --check` 通过。
- 本轮没有前端或训练逻辑改动，未再生成安装包；修复提交推送后的远程 Windows 构建仍需单独确认，不能以本地测试代表远程已成功。

## English summary

MCP now has 14 tools covering shared lightweight preflight, explicit user answers, generation/reshoot, identified resume, bounded execution evidence and saved-model export. Exports reuse the existing app-level Worker and edited virtual view, with unique project-local files, no-clobber publication, shared locks and export-specific SPZ consent. No training recipe, panorama High dispatch, project schema or cloud protocol change was made.

641 frontend tests, 7 diagnostic tests and 522 Rust tests passed (28 existing conditional ignores); build, formatting and strict Clippy passed. Real 2048-point SH3 compressed PLY/SPZ/SOG browser encoding and readback passed; Rust covers standard PLY. Separate SPZ normalization, cancellation and unsafe-scale checks passed with source hashes preserved. Native desktop MCP export UX, complete GPU generation, hardware fault handling and macOS/Linux runtime acceptance remain unverified.
