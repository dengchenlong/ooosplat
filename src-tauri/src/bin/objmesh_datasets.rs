//! Example CLI: assemble the route-A masked dual datasets (issue #8).
//!
//! Usage:
//!   objmesh_datasets <project_root> <colmap_model_dir> <masks_dir>
//!       [--out DIR] [--erode N] [--object-name NAME] [--background-name NAME]
//!
//! Builds `<out>/dataset-object` (white = object) and `<out>/dataset-background`
//! (white = everything else) from the extracted frames of `<project_root>` and
//! the per-frame SAM2 masks in `<masks_dir>`. `<out>` defaults to
//! `<project_root>/work/brush`. Prints the assembled layout plus per-frame mask
//! coverage for both sides, which is what the acceptance evidence uses.

use std::path::{Path, PathBuf};

use ooo_splat::pipeline::brush_datasets::{
    kept_pixels, prepare_datasets, transform_mask, DatasetSpec, MaskPolarity, MaskSpec,
};

const USAGE: &str = "usage: objmesh_datasets <project_root> <colmap_model_dir> <masks_dir> \
[--out DIR] [--erode N] [--object-name NAME] [--background-name NAME]";

struct Arguments {
    project_root: PathBuf,
    model_dir: PathBuf,
    masks_dir: PathBuf,
    out: Option<PathBuf>,
    erode_px: u8,
    object_name: String,
    background_name: String,
}

fn main() {
    let arguments = parse_arguments();
    let project_root = &arguments.project_root;
    let frames = project_root.join("work").join("frames");
    if !frames.is_dir() {
        eprintln!("frames directory missing: {}", frames.display());
        std::process::exit(1);
    }
    let out = arguments
        .out
        .clone()
        .unwrap_or_else(|| project_root.join("work").join("brush"));

    println!(
        "[1/3] reading sidecar masks from {}",
        arguments.masks_dir.display()
    );
    let specs = [
        DatasetSpec::masked(
            arguments.object_name.clone(),
            MaskPolarity::KeepObject,
            arguments.erode_px,
        ),
        DatasetSpec::masked(
            arguments.background_name.clone(),
            MaskPolarity::KeepBackground,
            arguments.erode_px,
        ),
    ];
    print_mask_coverage(&arguments.masks_dir, arguments.erode_px);

    println!(
        "[2/3] assembling {} datasets under {}",
        specs.len(),
        out.display()
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let datasets = runtime
        .block_on(prepare_datasets(
            &out,
            &frames,
            &arguments.model_dir,
            Some(&arguments.masks_dir),
            &specs,
        ))
        .expect("assemble datasets");

    println!("[3/3] verifying the layout Brush expects");
    for (spec, dataset) in specs.iter().zip(&datasets) {
        let images = count_files(&dataset.join("images"));
        let masks = count_files(&dataset.join("masks"));
        let sparse = ["cameras.bin", "images.bin", "points3D.bin"]
            .iter()
            .all(|name| dataset.join("sparse").join("0").join(name).is_file());
        println!(
            "      {}: images={} masks={} sparse={} -> {}",
            spec.name,
            images,
            masks,
            if sparse { "ok" } else { "MISSING" },
            dataset.display()
        );
        if images != masks || !sparse {
            eprintln!("dataset {} is not usable by Brush", spec.name);
            std::process::exit(1);
        }
    }
    println!("done");
}

fn print_mask_coverage(masks_dir: &Path, erode_px: u8) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(masks_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let object_spec = MaskSpec {
        polarity: MaskPolarity::KeepObject,
        erode_px,
    };
    let background_spec = MaskSpec {
        polarity: MaskPolarity::KeepBackground,
        erode_px,
    };
    let mut object_kept = 0_u64;
    let mut background_kept = 0_u64;
    let mut total = 0_u64;
    for path in &entries {
        let mask = match image::open(path) {
            Ok(mask) => mask.into_luma8(),
            Err(error) => {
                eprintln!("cannot read mask {}: {error}", path.display());
                std::process::exit(1);
            }
        };
        object_kept += kept_pixels(&transform_mask(&mask, object_spec));
        background_kept += kept_pixels(&transform_mask(&mask, background_spec));
        total += mask.width() as u64 * mask.height() as u64;
    }
    let frames = entries.len();
    println!(
        "      masks: {frames}; object keep {:.4} of frame area; background keep {:.4}; erode={erode_px}px",
        object_kept as f64 / total.max(1) as f64,
        background_kept as f64 / total.max(1) as f64,
    );
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_file())
                .count()
        })
        .unwrap_or(0)
}

fn parse_arguments() -> Arguments {
    let mut args = std::env::args().skip(1);
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut out = None;
    let mut erode_px = 1_u8;
    let mut object_name = "dataset-object".to_string();
    let mut background_name = "dataset-background".to_string();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "--erode" => {
                erode_px = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("--erode needs a 0..=8 pixel radius");
                        std::process::exit(2);
                    })
            }
            "--object-name" => {
                object_name = args.next().unwrap_or_else(|| {
                    eprintln!("--object-name needs a value");
                    std::process::exit(2);
                })
            }
            "--background-name" => {
                background_name = args.next().unwrap_or_else(|| {
                    eprintln!("--background-name needs a value");
                    std::process::exit(2);
                })
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with("--") => {
                eprintln!("unknown flag {other}\n{USAGE}");
                std::process::exit(2);
            }
            other => positional.push(PathBuf::from(other)),
        }
    }
    if positional.len() != 3 {
        eprintln!("{USAGE}");
        std::process::exit(2);
    }
    let masks_dir = positional.pop().expect("masks dir");
    let model_dir = positional.pop().expect("model dir");
    let project_root = positional.pop().expect("project root");
    Arguments {
        project_root,
        model_dir,
        masks_dir,
        out,
        erode_px,
        object_name,
        background_name,
    }
}
