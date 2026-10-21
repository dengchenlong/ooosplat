# 预览导航：平移、漫游与定位视角

此文档对应 `feat/panorama-video-v1` 开发分支，不代表已发布安装包。

## 使用方式

- 点击预览画布或“自动定位视角”后即可使用 W/S 沿水平面前后移动、A/D 沿水平面左右移动。方向随相机水平朝向变化，不受俯仰或侧倾影响，相机与观察中心高度保持不变；垂直朝上／朝下时继续使用当前水平朝向。基础速度为每秒当前可见画面高度的15%（原倍率的15%），随滚轮放大减速、缩小加速，Shift 三倍加速；相同距离和 FOV 下倍率一致。只在“调整”模式的透视视图中生效；正交视图不响应、不自动转为透视。
- 输入框、弹窗、导出、选择手势和操作轴拖动不会触发漫游；释放按键、失焦、切换工具或离开页面时停止。一次连续漫游结束后保存视角，不逐帧写本地存储。
- 三视图旁的纯文字按钮“自动定位视角”始终恢复首帧训练参考相机的位置、完整朝向及透视视场；从正视／侧视／顶视或自由正交视角点击，也会退出正交投影并取消三视图选中，不再仅居中原点并缩放。定位可以被鼠标、滚轮或键盘操作打断。
- 打开项目仍优先恢复上次本机视角。定位不改模型、不产生编辑撤销记录。
- 右上角统一为“图标 + 按键 + 功能”，如“左键 旋转”“右键 平移”“滚轮 缩放”“WASD 漫游”；不再展开 Shift 加速或透视限制说明，正交模式下漫游仍灰显且不生效。定位同步将焦点交还画布，动画结束不会抢走后来打开的输入框或弹窗焦点。

## 参考相机来源

普通任务从实际传给 Brush 的 `work/brush/dataset/sparse/0` 读取，支持二进制和文本 COLMAP 相机／姿态。按当前固定 Brush 加载器的文件名顺序选择首张有效训练图；OOOSplat 当前未传帧下采样或评估集分割参数，不以随机训练批次定义首帧。参考：[固定版本的加载器](https://github.com/ooolabdev/OOOBrush/blob/866a7d65b4d6a591ba0d52a88f8b20ea3465c1e5/crates/brush-dataset/src/formats/colmap.rs)。

全景任务读取 `work/panorama-training/manifest.json`，按实际时间戳选择最早已注册全景帧的 `front` 面，再读取训练稀疏模型中对应的针孔相机；不使用按名字排在前面的 `back` 面，也不把 ERP 的球面内参用于透视预览。

读取过程只保留相机及当前候选姿态，二进制二维观测通过 seek 跳过，文本观测行不整行分配；不扫描 Gaussian 文件。模型文件、JSON、名称和计数设有上限，错误不会阻止预览。

COLMAP 的 `R,t` 转换为相机中心 `-Rᵀt`、前方 `Rᵀ(0,0,1)`、上方 `Rᵀ(0,-1,0)`，随后应用与 Gaussian 一致的 PLY→引擎映射和编辑 Transform。保存完整朝向（含 roll）、视场、主点偏移和横纵焦距比例；不模拟镜头畸变。窗口宽高比不同，横向可见范围可以不同。

定位不再为三视图单独计算正交缩放，而是统一恢复真实透视相机，包括横纵焦距比例和主点偏移。轨道控制的观察距离仍采用缓存稳健中心的正深度，深度无效时以稳健包围盒半径尺度兜底；该距离只决定观察中心和交互尺度，不改变首帧相机位置、朝向或视场，不在定位时重新扫描 Gaussian。

缺少训练数据、损坏或清理过的项目禁用定位。外部导入模型即使附带 COLMAP，也不能证明 Gaussian 与相机坐标一致，因此禁用定位，不猜测视角或退化成包围盒适配。

## 平移倍率与兼容

右键平移使用 CSS 视口尺寸和当前投影尺度：正交为 `2 × orthoHeight / height`；透视为 `2 × distance × tan(fovY/2) / height`，横向同时考虑焦距比例。正交安全裁剪引起的相机后退不改变倍率，原网格大小和正近裁剪面保持不变。

本机视角继续使用原 version 1 存储键，新字段均可选；旧记录使用默认 roll／投影偏移和 52° FOV。视频捕获同步保留朝向与裁切后的视场，离线 HTML 的 Rust 校验和查看器同步接受新字段。不修改项目 schema、训练数据或配方。

## 历史验证记录（2026-10-19）

下列早期记录中的“三视图保持正交／居中原点／恢复参考缩放”是当时规则，已由当前统一恢复首帧透视相机的行为替代。

- 自动测试覆盖首次打开／恢复视角后连续放大再平移、正交裁剪退让、三视图极点、完整朝向恢复、WASD 方向及对角速度、Shift、失焦／取消／禁用、定位打断、旧记录与 HTML 参数兼容。
- Rust 对真实普通项目读取到 `frame_000002.jpg`，对已有真实全景结果读取到 `erp_0000000001/front.png`。两个项目均只读，没有重新生成或修改源数据。
- 使用真实完整 PLY 和生产 `GaussianViewer` 组件在隔离 Edge 中验证：全景 WebGL2 中文、全景 WebGPU 英文（含 1100px 最小窗口）、普通项目 WebGL2 英文 1280px 窗口。Tauri IPC 使用官方 mock；模型加载、GPU 渲染、相机控制、视角存储和界面布局为真实实现，不能等同于完整原生 WebView2 人工验收。
- 全景相机位置误差约 `2.74e-7`、普通项目约 `1.61e-7`；方向点积约 1。连续滚轮缩放后的实测平移与公式一致；三视图保持正交且原点居中，松键停止，英文窄窗口工具栏没有重叠。
- 证据：`.cache/preview-navigation/*/{proof.json,preview.png,browser.log}`。当前环境的浏览器 GPU 子进程需要沙箱外运行许可；未关闭浏览器自身的安全沙箱。
- 最终自动验证：399 项前端、6 项诊断接收端和 363 项常规 Rust 测试通过，前端构建、Rust 格式检查及 Clippy 通过。8 项需显式资源的 Rust 测试默认忽略，其中本次参考相机真实项目测试已另行执行；另有 2 项既有符号链接测试因当前 Windows 缺少创建符号链接权限而明确排除，未计入通过数。
- 仍需人工检查原生 Windows WebView2、真实鼠标／键盘长时间使用、不同 DPI 和更多模型；macOS／Linux 原生交互未实测。未重新执行生成或训练，也未把这次导航测试表述为训练效果验证。

### 同日降速与按钮调整复核（上一轮，降速已撤销）

- 漫游速度调整为可见画面高度的 15%／秒；自动测试覆盖缩放、FOV、窗口高度以及 30／60／144 FPS 下的速度，Shift 仍为三倍，右键平移倍率不变。
- 在真实全景 PLY 上重新执行生产预览检查：WebGPU 英文 1100px、WebGL2 中文 1280px 均通过。确认“自动定位视角／Auto locate view”为无图标纯文字按钮、工具栏无重叠、缩放后平移符合公式、漫游满足新速度上限、松键停止及正交视图不响应 WASD。
- 证据目录：`.cache/preview-navigation/1792378948112-webgpu-en/` 和 `.cache/preview-navigation/1792378948112-webgl2-zh/`，均包含 `proof.json`、`preview.png` 和 `browser.log`。本次自动回归为 416 项前端、6 项诊断及 365 项常规 Rust 测试，构建、格式检查、Clippy 通过；上述原生桌面及其他平台人工验证限制不变。

### 同日恢复倍率、焦点及参考缩放复核（当前实现）

- 恢复每秒一个可见画面高度的 WASD 倍率，实时读取距离与 FOV；定位过渡同时更新内部距离。修复按键前已排队的旧动画帧使漫游时钟倒退、随后重复计算位移的问题。自动测试覆盖缩放／旋转／恢复／定位后同倍率、30／60／144 FPS、Shift、失焦与输入隔离。
- 有效定位同步聚焦画布，动画期间可直接按 WASD 中断；动画完成不再聚焦，不抢走后来输入框的焦点。正交视图的提示保留但灰显，明确仅透视可用。
- 三视图改用缓存稳健中心，保留参考相机横纵焦距比例。含离群点、180° 坐标映射、旋转／缩放以及大范围网格安全后退的测试通过，后退不改变像素尺度；视角存储继续保留新的正交尺度与焦距比例。
- 真实普通项目 `20261011-234325_toyWithMask` 只读取得首张训练图 `frame_0000000000.png`，参考深度约 `7.99287`，三视图 `orthoHeight` 均约 `4.16745`；1280px WebGL2 中实测每单位约 `107.139984` 像素，与参考平面理论值 `107.139984` 一致。普通项目另通过 WebGPU 检查。
- 真实全景结果在 WebGPU／英文／精确 1100 CSS 像素视口、WebGL2／中文／1280px 下通过；两者均确认按钮点击后无需再点击画布即可漫游、松键停止、原点居中、提示未裁切。该全景 front 参考方向的稳健中心深度约 `−9.85203`，因此使用约定的稳健包围盒尺度兜底，而非声称精确匹配首帧主体占比。
- 最新截图与数值：`.cache/preview-navigation/1792381697060-webgpu-en/`（全景）、`1792381740304-webgl2-zh/`（全景）、`1792381742911-webgl2-en/`（普通）；普通 WebGPU 记录在 `1792381625421-webgpu-zh/`。验证脚本现在通过浏览器协议设置真实 CSS 视口，避免操作系统窗口边框使最小宽度截图被裁切。
- 最终单独执行的 `npm test` 为 427 项前端和 6 项诊断测试通过，构建通过。此前与两个软件渲染窗口并行执行的一轮出现超时及连带失败，渲染退出后的完整重跑通过，未修改测试超时来掩盖。另有 11 项 Rust HTML 导出测试与 1 项真实参考相机测试通过，1 项 HTML 浏览器夹具测试默认忽略；Rust 格式检查及 Clippy 通过。本轮没有修改 Rust 代码，也没有重新生成／训练项目。
- 生产预览和真实 GPU 渲染在隔离 Edge 中执行，Tauri IPC 为官方 mock；保存重开兼容由自动测试验证。原生 Windows WebView2 人工操作及 macOS／Linux 仍未验收。

### 提示精简复核

- 旋转提示统一为“左键 旋转”（矩形选择仍按实际操作显示“中键 旋转”），漫游合并为单项“图标 + WASD + 漫游”，移除展开的 Shift 加速和透视限制文字；正交灰显及实际操作规则不变。
- 427 项前端、6 项诊断测试及构建通过；实际生产预览在精确 1100px 视口下完成中英文图标／按键／功能顺序及无溢出检查。截图与断言证据：`.cache/preview-navigation/1792388890265-webgpu-zh/`、`1792388900006-webgpu-en/`。原生桌面人工验收限制不变。

## 当前定位规则复核：三视图返回首帧透视相机

- 正视／侧视／顶视点击“自动定位视角”，统一退出正交投影并恢复实际首帧训练相机，清除三视图选中；保留既有短时过渡、打断、焦点交还与视角保存。不修改模型或训练数据，历史保存的正交视角仍可按原样打开。
- 430 项前端、6 项诊断测试与构建通过；单元测试覆盖三视图即时／动画定位、坐标映射与模型变换、主点偏移／焦距比例、恢复保存状态，以及定位后直接漫游。
- 普通项目 WebGPU／中文／1100px 及全景项目 WebGL2／英文／1280px 的生产预览检查通过，三个入口相机位置误差分别约 `1.55e-7`、`2.74e-7`，方向点积为 1，FOV 分别恢复为约 `55.07466°` 和 `92°`。均确认定位前正交不响应 WASD，定位后无须点击画布即可漫游。
- 证据：`.cache/preview-navigation/1792394606311-webgpu-zh/`、`1792394624123-webgl2-en/`。首轮浏览器检查在对齐动画尚未结束时测量静止位置而失败，等待对齐结束后重跑通过；没有为此改变相机操作规则。隔离 Edge 的真实渲染不等同于原生 WebView2 人工验收，本轮未重新生成或训练、未执行新的 Rust 回归。

### 复现真实项目检查

```powershell
# 所有输出放在本地验证缓存，项目仅只读。
$env:OOOSPLAT_REFERENCE_PROJECT = '<已完成项目目录>'
$env:OOOSPLAT_REFERENCE_ARTIFACT = 'E:/Project/OOOSplat/.cache/panorama-v1/reference-view.json'
cargo test --manifest-path src-tauri/Cargo.toml --lib actual_training_reference_view -- --ignored --nocapture
node scripts/verify-preview-navigation.mjs '<已完成项目目录>' '.cache/panorama-v1/reference-view.json' webgl2 zh
node scripts/verify-preview-navigation.mjs '<已完成项目目录>' '.cache/panorama-v1/reference-view.json' webgpu en 1280
```

本地 Edge 验证脚本使用独立配置目录，不访问个人浏览器资料，不运行导出或编辑写入。

## English summary

Perspective-only WASD navigation stays on the world-horizontal XZ plane, following camera yaw independently of pitch/roll (including vertical views), and preserves camera/target height. Speed is 15% of one visible height per second, zoom-relative, with Shift ×3. Clicking the canvas or the text-only “Auto locate view” button enables it directly in Adjust; locating returns focus synchronously, never at animation completion. Upper-right hints remain visible but disabled in orthographic views. Queued timestamps cannot move the flight clock backwards or double-count movement. Ordinary projects use their actual first valid Brush dataset image; panorama projects use the earliest registered ERP frame's front training face. Locate always restores that perspective camera, leaving any fixed/free orthographic view and clearing fixed-view selection. The cached robust depth (or bounds fallback) sets only the orbit target/distance, not a substitute orthographic zoom or camera pose. Imported/missing/unverified cameras disable locating. Right-button pan is unchanged. Roll, FOV and projection state remain compatible with saved views, video capture and offline HTML. Earlier speed and orthographic-scale verification records above describe superseded behavior.

Earlier reference-view verification (before the horizontal-movement change) passed 427 frontend tests, 6 diagnostic tests, build, 11 Rust HTML-export tests, a separate real-project reference-camera test, formatting and Clippy. One optional HTML browser-fixture test remained ignored. A test run concurrent with software-rendered browser checks timed out and cascaded; the full isolated rerun passed without increasing timeouts. Real ordinary/panorama PLYs passed production Edge WebGL2/WebGPU checks for direct post-locate flight and orthographic pixel scale; the panorama sample exercised the invalid-depth fallback. The English narrow-layout check used an exact 1100px CSS viewport. Tauri IPC was mocked; native WebView2 manual and non-Windows platform checks remain outstanding. No generation/training was rerun.

Horizontal-navigation update (2026-10-19): 475 frontend tests (including 58 camera-control tests), 6 diagnostic tests and the production build passed. Coverage includes four ground-plane directions, pitch/roll and exact vertical views, unchanged camera height/orientation, 15% speed, zoom/FOV scaling, 30/60/144 FPS, normalized diagonals, opposing keys, Shift, focus isolation and immediate post-locate navigation. Movement translates the camera/target without recomputing lookAt, avoiding roll jitter at vertical views. This update did not rerun real-model browser/native rendering or generation/training; prior rendering results are not new validation of horizontal movement. Existing browser externalization/WASM URL/chunk-size build warnings remain.
