//! Offline M1 mesh extraction CLI (issue #10).
//!
//! Usage:
//!   objmesh_mesh <project_root> <colmap_model_dir> --out <file.glb>
//!       [--voxel-size S | --resolution N] [--truncation-voxels V]
//!       [--min-observations K] [--depth-scale D] [--max-frames N]
//!       [--scale-mm F] [--color-cell-size C]
//!
//! Reads `<project_root>/assets/object.ply` (read-only) plus the COLMAP
//! sparse model, renders one depth map per frame from the object splats,
//! fuses them into a TSDF, extracts the zero level set and writes a
//! vertex-coloured GLB. The last line on stdout is a JSON summary
//! (`splats`, `frames`, `voxels_fused`, `vertices`, `triangles`, `bbox`,
//! `object_bbox`, `voxel_size`, `out`).

use std::path::PathBuf;

use ooo_splat::reconstruction::mesh::{build_object_mesh, write_mesh_glb, MeshOptions};

const USAGE: &str = "usage: objmesh_mesh <project_root> <colmap_model_dir> --out <file.glb> \
[--voxel-size S | --resolution N] [--truncation-voxels V] [--min-observations K] \
[--depth-scale D] [--max-frames N] [--scale-mm F] [--color-cell-size C]\n\
\n\
  --voxel-size S        TSDF voxel edge in scene units (default: object AABB longest edge / --resolution)\n\
  --resolution N        voxels along the longest AABB edge when --voxel-size is absent (default 256)\n\
  --truncation-voxels V truncation band in voxels (default 4)\n\
  --min-observations K  drop voxels seen by fewer than K frames (default 2)\n\
  --depth-scale D       render depth at 1/D resolution (default 2)\n\
  --max-frames N        fuse only the first N frames (default: all)\n\
  --scale-mm F          rescale the longest mesh edge to F millimetres (default: keep COLMAP scale)\n\
  --color-cell-size C   nearest-splat colour lookup cell (default: 2 voxels)";

struct Arguments {
    project_root: PathBuf,
    model_dir: PathBuf,
    out: PathBuf,
    options: MeshOptions,
}

fn parse_arguments() -> Arguments {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut out = None;
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
    let out = out.unwrap_or_else(|| fail("--out is required"));
    if options.voxel_size.is_some() && resolution_given {
        // Both accepted; --voxel-size simply wins. Say so once instead of
        // silently ignoring the other flag.
        println!("note: --voxel-size given, --resolution is ignored");
    }
    Arguments {
        project_root,
        model_dir,
        out,
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
        out,
        options,
    } = arguments;
    println!(
        "[1/3] extracting object mesh from {}/assets/object.ply",
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

    println!("[3/3] writing {}", out.display());
    let summary = match write_mesh_glb(&out, &build) {
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
