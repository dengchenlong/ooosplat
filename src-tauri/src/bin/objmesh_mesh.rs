//! Offline M1 mesh extraction CLI (issue #10).
//!
//! Usage:
//!   objmesh_mesh <project_root> <colmap_model_dir> --out <file.glb>
//!       [--voxel-size S | --resolution N] [--truncation-voxels V]
//!       [--min-observations K] [--depth-scale D] [--max-frames N]
//!       [--scale-mm F] [--color-cell-size C]
//!   objmesh_mesh <project_root> <colmap_model_dir> --publish
//!       [same mesh options]
//!
//! Reads the project's object splat cloud read-only
//! (`assets/object-XX.ply`, falling back to the pre-#11 `assets/object.ply`)
//! plus the COLMAP sparse model, renders one depth map per frame from the
//! object splats, fuses them into a TSDF, extracts the zero level set and
//! writes a vertex-coloured GLB. The last line on stdout is a JSON summary
//! (`splats`, `frames`, `voxels_fused`, `vertices`, `triangles`, `bbox`,
//! `object_bbox`, `voxel_size`, `out`).
//!
//! `--publish` writes the GLB atomically to `<project_root>/assets/object-01.glb`
//! and records it in `assets/manifest.json` (issue #11); `out` then names that
//! published path. `--out` and `--publish` are mutually exclusive.

use std::path::PathBuf;

use ooo_splat::reconstruction::mesh::{
    build_object_mesh, parse_mm_ref, publish_mesh_glb, write_mesh_glb, MeshOptions, MeshSummary,
};

const USAGE: &str = "usage: objmesh_mesh <project_root> <colmap_model_dir> (--out <file.glb> | --publish) \
[--voxel-size S | --resolution N] [--truncation-voxels V] [--min-observations K] \
[--depth-scale D] [--max-frames N] [--scale-mm F] [--color-cell-size C] \
[--min-component-ratio R] [--keep-largest-only] [--target-triangles N | --simplify-ratio R] \
[--scale-from-mm-ref A:B]\n\
\n\
  --out FILE.glb        write the GLB to this path\n\
  --publish             publish to <project_root>/assets/object-01.glb and record it in assets/manifest.json\n\
  --voxel-size S        TSDF voxel edge in scene units (default: object AABB longest edge / --resolution)\n\
  --resolution N        voxels along the longest AABB edge when --voxel-size is absent (default 256)\n\
  --truncation-voxels V truncation band in voxels (default 4)\n\
  --min-observations K  drop voxels seen by fewer than K frames (default 2)\n\
  --depth-scale D       render depth at 1/D resolution (default 2)\n\
  --max-frames N        fuse only the first N frames (default: all)\n\
  --scale-mm F          rescale the longest mesh edge to F millimetres (default: keep COLMAP scale)\n\
  --color-cell-size C   nearest-splat colour lookup cell (default: 2 voxels)\n\
  --min-component-ratio R  drop components holding less than R of the triangles (0..1; not set = keep every component)\n\
  --keep-largest-only  keep only the largest connected component (wins over --min-component-ratio)\n\
  --target-triangles N  simplify the mesh down to at most N triangles\n\
  --simplify-ratio R    simplify to R of the extracted triangles (0..1; --target-triangles wins if both are given)\n\
  --scale-from-mm-ref A:B  calibrate physical scale: the mesh span A (scene units) is known to be B millimetres";

struct Arguments {
    project_root: PathBuf,
    model_dir: PathBuf,
    /// `Some(path)` for `--out`, `None` for `--publish`.
    destination: Destination,
    options: MeshOptions,
}

/// Where the GLB goes: a caller-named file or the project's published slot.
enum Destination {
    Out(PathBuf),
    Publish,
}

impl Destination {
    /// The path the summary reports, for the progress line before the write.
    fn describe(&self, project_root: &std::path::Path) -> String {
        match self {
            Self::Out(path) => path.display().to_string(),
            Self::Publish => project_root
                .join("assets")
                .join(ooo_splat::reconstruction::assets::object_glb_name(
                    ooo_splat::reconstruction::assets::FIRST_OBJECT_INDEX,
                ))
                .display()
                .to_string(),
        }
    }

    fn write(
        &self,
        project_root: &std::path::Path,
        build: &ooo_splat::reconstruction::mesh::MeshBuild,
    ) -> ooo_splat::error::Result<MeshSummary> {
        match self {
            Self::Out(path) => write_mesh_glb(path, build),
            Self::Publish => publish_mesh_glb(project_root, build),
        }
    }
}

fn parse_arguments() -> Arguments {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut out = None;
    let mut publish = false;
    let mut options = MeshOptions::default();
    let mut resolution_given = false;
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        let mut value = || -> String {
            index += 1;
            arguments
                .get(index)
                .cloned()
                .unwrap_or_else(|| fail(&format!("{argument} requires a value")))
        };
        match argument {
            "--out" => out = Some(PathBuf::from(value())),
            "--publish" => publish = true,
            "--voxel-size" => options.voxel_size = Some(parse_number(&value(), "--voxel-size")),
            "--resolution" => {
                options.resolution = parse_usize(&value(), "--resolution");
                resolution_given = true;
            }
            "--truncation-voxels" => {
                options.truncation_voxels = parse_number(&value(), "--truncation-voxels");
            }
            "--min-observations" => {
                options.min_observations = parse_usize(&value(), "--min-observations") as u32;
            }
            "--depth-scale" => {
                options.depth_scale = parse_usize(&value(), "--depth-scale") as u32;
            }
            "--max-frames" => {
                options.max_frames = Some(parse_usize(&value(), "--max-frames"));
            }
            "--scale-mm" => options.scale_mm = Some(parse_number(&value(), "--scale-mm")),
            "--color-cell-size" => {
                options.color_cell_size = Some(parse_number(&value(), "--color-cell-size"));
            }
            "--min-component-ratio" => {
                options.min_component_ratio = Some(parse_number(&value(), "--min-component-ratio"));
            }
            "--keep-largest-only" => options.keep_largest_only = true,
            "--target-triangles" => {
                options.target_triangles = Some(parse_usize(&value(), "--target-triangles"));
            }
            "--simplify-ratio" => {
                options.simplify_ratio = Some(parse_number(&value(), "--simplify-ratio"));
            }
            "--scale-from-mm-ref" => {
                let raw = value();
                options.scale_from_mm_ref = Some(parse_mm_ref(&raw).unwrap_or_else(|| {
                    fail(&format!(
                        "--scale-from-mm-ref expects <scene-length>:<millimetres>, got {raw}"
                    ))
                }));
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with("--") => fail(&format!("unknown option {other}")),
            other => positional.push(PathBuf::from(other)),
        }
        index += 1;
    }
    if positional.len() != 2 {
        fail("expected exactly two positional arguments: project root and COLMAP model dir");
    }
    let model_dir = positional.pop().expect("checked length");
    let project_root = positional.pop().expect("checked length");
    // `--out` and `--publish` are mutually exclusive; giving neither keeps the
    // original behaviour of requiring `--out`.
    let destination = match (out, publish) {
        (Some(_), true) => fail("--out and --publish are mutually exclusive"),
        (Some(path), false) => Destination::Out(path),
        (None, true) => Destination::Publish,
        (None, false) => fail("either --out <file.glb> or --publish is required"),
    };
    if options.voxel_size.is_some() && resolution_given {
        // Both accepted; --voxel-size simply wins. Say so once instead of
        // silently ignoring the other flag.
        println!("note: --voxel-size given, --resolution is ignored");
    }
    Arguments {
        project_root,
        model_dir,
        destination,
        options,
    }
}

fn parse_number(text: &str, flag: &str) -> f64 {
    text.parse::<f64>()
        .unwrap_or_else(|_| fail(&format!("{flag} expects a number, got {text}")))
}

fn parse_usize(text: &str, flag: &str) -> usize {
    text.parse::<usize>()
        .unwrap_or_else(|_| fail(&format!("{flag} expects a positive integer, got {text}")))
}

fn fail(message: &str) -> ! {
    eprintln!("{message}\n{USAGE}");
    std::process::exit(2);
}

fn main() {
    std::process::exit(run(parse_arguments()));
}

fn run(arguments: Arguments) -> i32 {
    let Arguments {
        project_root,
        model_dir,
        destination,
        options,
    } = arguments;
    println!(
        "[1/3] extracting object mesh from {}/assets/object-XX.ply (or object.ply)",
        project_root.display()
    );
    let build = match build_object_mesh(&project_root, &model_dir, &options) {
        Ok(build) => build,
        Err(error) => {
            eprintln!("错误：{error}");
            return 1;
        }
    };
    println!(
        "      splats: {}; frames: {}; voxels fused: {}; voxel size: {:.5}",
        build.splats, build.frames, build.voxels_fused, build.voxel_size
    );
    let (min, max) = build.object_bounds;
    println!(
        "      object AABB: [{:.3}, {:.3}, {:.3}] .. [{:.3}, {:.3}, {:.3}]",
        min[0], min[1], min[2], max[0], max[1], max[2]
    );

    println!("[2/3] extracting the zero level set");
    println!(
        "      vertices: {}; triangles: {}",
        build.mesh.positions.len(),
        build.mesh.triangle_count()
    );
    if build.mesh.is_empty() {
        eprintln!("错误：TSDF 没有提取到任何表面，请检查物体泼溅与相机的对齐情况");
        return 1;
    }

    println!("[3/3] writing {}", destination.describe(&project_root));
    let summary = match destination.write(&project_root, &build) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("错误：{error}");
            return 1;
        }
    };
    println!("      bbox: {:?} .. {:?}", summary.bbox[0], summary.bbox[1]);
    // The orchestration side reads the final line, so it must be the JSON.
    match serde_json::to_string(&summary) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(error) => {
            eprintln!("错误：无法序列化摘要：{error}");
            1
        }
    }
}
