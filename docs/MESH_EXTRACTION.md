# M1 物体 mesh 提取（issue #10）

`objmesh_mesh` 把 `<project>/assets/object.ply` 的物体泼溅 + COLMAP 相机
→ 逐帧深度图 → TSDF 融合 → 零交叉提取 → 顶点色 `.glb`。纯 Rust，无新增依赖。

## 用法

```bash
cargo build --release --bin objmesh_mesh
src-tauri/target/release/objmesh_mesh <project_root> <colmap_model_dir> --out <file.glb> \
    [--voxel-size S | --resolution N] [--truncation-voxels V] [--min-observations K] \
    [--depth-scale D] [--max-frames N] [--scale-mm F] [--color-cell-size C]
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

stdout 最后一行是 JSON 摘要，便于编排方校验：

```json
{"splats":6927,"frames":276,"voxels_fused":128662,"vertices":104231,
 "triangles":194740,"bbox":[[...],[...]],"object_bbox":[[...],[...]],
 "voxel_size":0.05,"out":"..."}
```

`bbox` 是 mesh 顶点范围（`--scale-mm` 之后），`object_bbox` 是源泼溅 AABB
（场景单位），两者量级一致说明几何没有跑飞。

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

## 偏差与未做项

- **没有在 Blender 里打开验证**：本机未安装 Blender。验收改用
  `scripts/verify-glb.py` 做容器与数据层核验（见上），并额外用 z-buffer
  视图渲染过三个正交方向的轮廓图；「Blender 正常打开、目视几何误差可接受」
  这一项仍需有 Blender 的环境补做。
- **物体边缘的薄片残片**：P2 的 mesh 有 174 个连通分量，最大分量占
  194740 个三角形中的 111614（57%）。残片来自物体泼溅本身在边缘的孤立簇
  （拆分阶段的身体/地面残留），TSDF 会如实把它们建出来。若要单壳输出，
  后续可加「保留最大连通分量」或按连通分量体积过滤的开关。
- **不做 mesh 简化/平滑**：0.05 体素下 P2 是 19 万三角形，直接进 Blender
  可用但偏重；简化需要额外的边折叠实现，本期没做。
- **只用 DC 颜色**：忽略 SH 高阶项，因此顶点色是视角无关的基色。
- license：本条走纯 Rust 自研，未引入 Open3D（issue 正文里提到的 MIT 依赖），
  所以 `licenses/` 无需新增条目。