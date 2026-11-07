# M1 物体 mesh 提取（issue #10）

`objmesh_mesh` 把 `<project>/assets/object.ply` 的物体泼溅 + COLMAP 相机
→ 逐帧深度图 → TSDF 融合 → 零交叉提取 → 顶点色 `.glb`。纯 Rust，无新增依赖。

## 用法

```bash
cargo build --release --bin objmesh_mesh
src-tauri/target/release/objmesh_mesh <project_root> <colmap_model_dir> --out <file.glb> \
    [--voxel-size S | --resolution N] [--truncation-voxels V] [--min-observations K] \
    [--depth-scale D] [--max-frames N] [--scale-mm F] [--color-cell-size C] \
    [--min-component-ratio R] [--keep-largest-only] \
    [--target-triangles N | --simplify-ratio R] [--scale-from-mm-ref A:B]
```

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `--voxel-size S` | — | TSDF 体素边长（场景单位）。省略时按「物体 AABB 最长边 / `--resolution`」推导 |
| `--resolution N` | 256 | 未给 `--voxel-size` 时，最长边上的体素数 |
| `--truncation-voxels V` | 4 | 融合带宽（体素），实际截断距离 = `V * voxel_size` |
| `--min-observations K` | 2 | 少于 K 帧观测到的体素不参与提取 |
| `--depth-scale D` | 2 | 深度图按 1/D 分辨率渲染（540x960 → 270x480） |
| `--max-frames N` | 全部 | 只融合模型顺序的前 N 帧 |
| `--scale-mm F` | 不带 | 把 mesh 最长边缩放到 F 毫米；不带时保持 COLMAP 原尺度 |
| `--color-cell-size C` | 2 体素 | 最近泼溅取色的空间哈希格子 |
| `--min-component-ratio R` | 不带 | 丢弃三角形占比小于 R（0..1）的连通分量；不传＝保留全部分量，输出与 #10 逐字节一致 |
| `--keep-largest-only` | 关 | 只保留最大连通分量（优先于 `--min-component-ratio`） |
| `--target-triangles N` | 不带 | 简化到最多 N 个三角形 |
| `--simplify-ratio R` | 不带 | 简化到提取结果的 R 倍（0..1]。与 `--target-triangles` 同时给时后者生效） |
| `--scale-from-mm-ref A:B` | 不带 | 尺度标定：mesh 上量得 A（场景单位）实际为 B 毫米，整体缩放到该长度＝B 毫米 |

stdout 最后一行是 JSON 摘要，便于编排方校验：

```json
{"splats":6927,"frames":276,"voxels_fused":128662,"vertices":104231,
 "triangles":194740,"bbox":[[...],[...]],"object_bbox":[[...],[...]],
 "voxel_size":0.05,"components":174,"components_removed":0,
 "vertices_before_simplify":null,"triangles_before_simplify":null,
 "scale_factor":1.0,"units":"scene","out":"..."}
```

`bbox` 是 mesh 顶点范围（`--scale-mm` / `--scale-from-mm-ref` 之后），
`object_bbox` 是源泼溅 AABB（场景单位），两者量级一致说明几何没有跑飞。
`components` 是最终 mesh 的连通分量数，`components_removed` 是清理阶段丢掉
的分量数，`vertices_before_simplify` / `triangles_before_simplify` 只在简化
跑过时非 `null`，`scale_factor` 是实际施加的等比缩放（不缩放为 `1.0`），
`units` 为 `scene`（COLMAP 原尺度）或 `m`（已标定）。

## 流水线

1. **逐帧深度**（`reconstruction/mesh/depth.rs`）：每个泼溅按
   `CameraProjection` 投影，用 `footprint_radius_px` 得到像素半径画成
   等深圆盘写入 per-pixel min 深度。`fill_holes` 补 1-2 px 针孔（只在四邻
   深度接近时填，不跨越轮廓），`smooth_depth` 用受限中值去掉圆盘台阶。
2. **TSDF 融合**（`tsdf.rs`）：以物体 AABB + 截断边距建体素网格，
   `sdf = depth - voxel_depth`，仅 `|sdf| < truncation` 时加权平均；观测
   不足 `--min-observations` 的体素保持 `NaN`，提取阶段绝不会凭空补面。
3. **表面提取**（`surface.rs`）：每个体素立方体用 Kuhn 6 四面体分解，逐四
   面体求零交叉（等价 marching cubes 的等值面，但无需 256 案例表）；跨体素
   共享的网格边去重，内部边恰好被两个三角形共用，因此水密。法线取 TSDF 梯
   度，最后统一翻转到 glTF 的逆时针正面。
4. **顶点色**：`ply.rs::stream_colored_splats` 读 `f_dc_0..2`，
   `0.5 + SH_C0 * f_dc` 转线性 RGB 并 clamp；顶点用空间哈希找最近泼溅取色。
5. **GLB**（`glb.rs`）：手写 glTF 2.0 容器（JSON chunk 空格补齐、BIN chunk
   零补齐、均 4 字节对齐），POSITION/NORMAL/COLOR_0 为 float32 VEC3、索引
   uint32、`mode=4`，POSITION accessor 带 min/max。
6. **收尾**（`cleanup.rs`，issue #17）：顶点并查集求三角形连通分量 →
   按占比过滤／只留最大分量；最短边优先的边折叠简化（折叠后顶点取中点、
   颜色取两端平均、法线按最终三角形重算）；`--scale-from-mm-ref` 用
   「量得长度 : 已知毫米」求等比因子，把整个 mesh 变成米制。

## 尺度标定（没有基准就没有尺寸）

**COLMAP 输出的坐标是没有物理单位的**：整体乘一个常数后所有相机和点云
的重投影误差完全不变，所以单看 mesh 无法判断理发椅是 6.65 单位还是 6.65 米。
`--scale-mm` 只是「把最长边当作用户给的长度」，属于主观赋值；真正有依据的
是 `--scale-from-mm-ref A:B`——在拍摄现场量一段已知长度（尺子/量块/工件图上
的标注尺寸都行），在 mesh 上量出同一段的场景单位长度 A，声明它等于 B 毫米。
程序给出 `scale_factor = (B / 1000) / A` 并把它乘到所有顶点上，此时
`units = "m"`，导出的 GLB 才有真实尺寸。

- 量哪一段最好：取外形上两点明确、易复现的距离（工件总长、两孔中心距）。
  mesh 顶点是 TSDF 的外扩包络，量外轮廓会比真实尺寸略大（P2 约 +0.3~0.4 场景
  单位，由截断带宽 4 体素 + 半格造成），所以基准要量同一类特征。
- 不标定时 `scale_factor = 1.0`、`units = "scene"`，坐标仍是 COLMAP 无量纲单位。
- 精度上限还是摄影测量本身（工件尺寸的 0.1%~0.5%），标定只解决「单位从哪来」，
  不会把误差变小。

## 验收

```bash
python scripts/verify-glb.py <file.glb>
```

不依赖 glTF 库或 Blender，检查：文件头与声明长度、两个 chunk 的对齐与顺序、
JSON 可解析且 `asset.version == "2.0"`、每个 accessor 落在 bufferView 与
BIN chunk 内且字节数匹配、索引不越界、POSITION min/max 与文件内实际浮点数
据一致、法线单位长度、颜色在 0..1。

真实数据实测（`--voxel-size 0.05`）：

| 项目 | splats | frames | vertices | triangles | mesh span |
| --- | --- | --- | --- | --- | --- |
| P2 理发椅 | 6927 | 276 | 104231 | 194740 | 6.65 / 6.07 / 5.90 |
| P1 手链架 | 13498 | 153 | 53604 | 101982 | 6.58 / 4.41 / 3.90 |

P2 源泼溅 AABB span 6.26 / 5.74 / 5.51，mesh 略大约 0.3-0.4，是截断边距
（4 体素 = 0.2）+ 体素半格的自然外扩。

## 真实数据验收（issue #17）

新增旗标在本机两份样例上的实测（release 二进制，`--voxel-size 0.05`）：

| 项目 | 命令 | 分量（清理前→后） | 三角形 | 备注 |
| --- | --- | --- | --- | --- |
| 回归锁 | 不带任何新旗标 | 174 | 194740 / 104231 顶点 | P2 GLB SHA256 与 #10 产物逐字节一致 |
| P2 | `--min-component-ratio 0.02` | 174 → **5** | 194740 → 186058（+ 3.0%） | 最大分量占比 0.573 → 0.600 |
| P2 | `--keep-largest-only` | 174 → **1** | → 111614 | 硬指标，量变最剧烈 |
| P2 | `--target-triangles 100000` | — | → **100000** | 整轮 38.0s（含简化） |
| P1 | `--min-component-ratio 0.02` | 46 → **2** | 101982 → 100004 | 最大分量占比 0.950 → 0.969 |

- **P2 的「≤10 分量 / 最大占比 ≥0.9」达不到**：`--min-component-ratio 0.02`
  下 P2 剩 5 个分量（≤10 ✔）但最大分量只占 0.600。这不是实现问题，是源数据
  有洞：最大分量 y 0.72~4.17，上面 4 块 y 3.94~6.79（对角 2.4~3.5、场景对角
  10.8），最大分量与第 2 大分量的最近点相距 **0.62**，而该最近点对的中点
  0.5 半径内 **一个源泼溅都没有**（`object.ply` 最近泼溅 0.51，而该云自身的
  最近邻间距 p90 只有 0.049、最大 0.100）。也就是说物体上半身与下半身之间在
  `assets/object.ply` 里就断开了 0.5 以上的缝，TSDF 如实把两截建成两个壳。
  要 0.9 只能把上半身整块删掉（占 40% 三角形），那是丢几何不是清理残片，
  所以这条对本样例不成立；issue 里的 0.9 是按「残片为主」预估的。要单壳输出
  用 `--keep-largest-only`（实测 174 → 1 个分量、111614 三角形）。
  P1 满足：46 → 2 个分量、最大占比 0.969，且没有外扩出新空洞（两视图轮廓
  IoU 0.986~1.000，`missing` 0.02%~1.5%）。
- **`--min-component-ratio` 的「默认」是关，不是 0.02**：issue 正文写「默认
  0.02」，但同时要求「不传新旗标时输出与 #10 逐字节一致」。两条不可兼得（0.02
  会删掉 P2 的 4.46% 三角形），所以取后者为硬约束：旗标不带 = 不清理。0.02 是
  推荐值，写在 `docs/MESH_EXTRACTION.md` 的参数表里。
- **P2 残片确实是残片**：占比 <0.02 被删掉的 169 个分量合计只占 4.46% 三角形，
  中位数 13 个三角形、最大 1044 —— 与「孤立泼溅簇融成的小壳」一致。
- **简化后轮廓不变（在「预览级」减面上）**：`--target-triangles 100000`
  （减到 51%）与 baseline 的三正交视图轮廓 IoU 0.991/0.991/0.995；
  `--min-component-ratio 0.02` 为 0.981/0.974/0.990（差的部分全是被删掉的小
  分量：`missing` 1~2.6%，`extra` = 0）。
- **减得越狠轮廓掉得越明显**（同一份 P2，纯简化、不做分量清理）：

  | `--target-triangles` | 实际三角形 | 轮廓 IoU（front/side/top） |
  | --- | --- | --- |
  | 100000 | 100000 | 0.992 / 0.991 / 0.995 |
  | 50000 | 49999 | 0.974 / 0.971 / 0.983 |
  | 20000 | 20000 | 0.921 / 0.900 / 0.938 |

  即「1/4 减面」还留得住轮廓，「1/10 减面」就开始削薄（`missing` 6~9%）——
  边折叠按最短边优先，密度低的地方先塌，薄壁件尤其明显。要 LOD 就从 5~10 万
  起，不要一步砍到 2 万。
- **`--simplify-ratio` 与组合用法**（同一份 P2）：`--simplify-ratio 0.5` →
  97369 三角形（目标 97370，差 1 是因为折叠会顺带吃掉邻接三角形）；
  `--min-component-ratio 0.02 --target-triangles 50000` 一次跑完 → 5 个分量、
  50000 三角形，摘要里 `components_removed=169`、`triangles_before_simplify=186058`
  （清理后的数，说明顺序是「先清理后简化」）。
- **标定数字核对**：P2 用 `--scale-from-mm-ref 6.65:1000`（量得 mesh 最长边
  6.65 场景单位 = 1 m）→ `scale_factor = 0.15037593984962405`，
  `(1000/1000)/6.65` 手算一致，输出 bbox span 由 6.65 变为 1.000，`units = "m"`。
- **零新增 crate**：全部 `--offline` 编译；`cargo test --lib` 656 passed /
  0 failed / 28 ignored（基线 641 + 新增 15 条）。
- **官方 glTF-Validator**：clean / simplify 两份新产物均 0 errors / 0 warnings。

用于核对分量与轮廓的临时脚本（不在仓库里，留在验收目录）：
`E:/ooosplat-accept-20261107/{glb_components.py, glb_components_geo.py,
glb_seams.py, glb_silhouette.py, glb_component_paint.py}`。

## 偏差与未做项

- **没有在 Blender 里打开验证**：本机未安装 Blender。验收改用
  `scripts/verify-glb.py` + 官方 glTF-Validator 做容器与数据层核验（见上），
  并额外用 z-buffer 视图渲染过三个正交方向的轮廓对比图；「Blender 正常打开、
  目视几何误差可接受」这一项仍需有 Blender 的环境补做。
- **简化是边折叠，不是 QEM**：按最短边优先折叠、顶点取中点，不做二次误差
  度量。目标三角形数下轮廓保持得很好（IoU ≥0.99），但没有做顶点位置最优解，
  高曲率处会略钝。
- **只用 DC 颜色**：忽略 SH 高阶项，因此顶点色是视角无关的基色。简化时新顶点
  的颜色取两端平均，不会跑出源色域。
- license：本条走纯 Rust 自研，未引入 Open3D（issue 正文里提到的 MIT 依赖），
  所以 `licenses/` 无需新增条目。