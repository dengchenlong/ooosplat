//! Example CLI: SAM2 mask back-projection voting + object/background split.
//!
//! Usage:
//!   objmesh_backproject <project_root> <colmap_model_dir> <masks_dir>
//!       [--min-cluster-points N] [--max-distance-ratio R|none]
//!
//! Reads `<project_root>/final.ply` (read-only), votes every splat against
//! the per-frame mask PNGs in `<masks_dir>`, clusters the candidates, filters
//! whole clusters that are too small or too far from the largest one
//! (`ClusterFilterConfig`, issue #15) and writes
//! `<project_root>/derived/instance.mask` plus `<project_root>/assets/object.ply`
//! / `background.ply`. Prints vote-ratio statistics and the per-cluster
//! decisions for acceptance evidence.

use std::path::PathBuf;

use ooo_splat::reconstruction::backproject::{
    filter_object_clusters, label_object_clusters, load_frame_views, vote_splats,
    write_instance_mask, ClusterConfig, ClusterFilterConfig, ClusterFilterOutcome, VoteConfig,
};
use ooo_splat::reconstruction::ply::stream_splat_cores;
use ooo_splat::reconstruction::splat_transform::export_instance_assets;

const USAGE: &str = "usage: objmesh_backproject <project_root> <colmap_model_dir> <masks_dir> \
[--min-cluster-points N] [--max-distance-ratio R|none]";

fn main() {
    let (project_root, model_dir, masks_dir, filter_config) = parse_arguments();

    let ply_path = project_root.join("final.ply");
    println!("[1/4] pairing COLMAP model with masks from {masks_dir:?}");
    let frames = load_frame_views(&model_dir, &masks_dir).expect("load frame views");
    let png_count = std::fs::read_dir(&masks_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "png"))
                .count()
        })
        .unwrap_or(0);
    println!(
        "      paired frames: {} of {} masks",
        frames.len(),
        png_count
    );

    println!("[2/4] voting every splat against the per-frame masks");
    let outcome = vote_splats(&ply_path, &frames, &VoteConfig::default(), 4096).expect("vote");
    let candidate_count = outcome.candidates.iter().filter(|c| **c).count();
    println!(
        "      splats: {}; candidates: {} ({:.2}%)",
        outcome.splat_count,
        candidate_count,
        100.0 * candidate_count as f64 / outcome.splat_count as f64
    );

    println!("[3/4] collecting positions and clustering candidates (eps=0.1, min_points=10)");
    let mut positions: Vec<[f64; 3]> = Vec::with_capacity(outcome.splat_count as usize);
    stream_splat_cores(&ply_path, 4096, &mut |batch| {
        positions.extend(batch.iter().map(|splat| splat.position));
        Ok(())
    })
    .expect("stream positions");
    assert_eq!(positions.len() as u64, outcome.splat_count);
    let clusters = label_object_clusters(
        &positions,
        &outcome.candidates,
        &ClusterConfig {
            eps: 0.1,
            min_points: 10,
        },
    );
    println!("      clusters: {}", clusters.cluster_count);

    println!(
        "      filtering clusters (min_cluster_points={}, max_distance_ratio={})",
        filter_config.min_cluster_points,
        filter_config
            .max_distance_ratio
            .map_or_else(|| "none".to_string(), |ratio| format!("{ratio}"))
    );
    let filtered = filter_object_clusters(&positions, &clusters.labels, &filter_config)
        .expect("filter clusters");
    print_cluster_report(&filtered);
    let labels = &filtered.labels;

    println!("[4/4] writing instance.mask and splitting assets");
    let object_count = write_instance_mask(
        &project_root.join("derived").join("instance.mask"),
        outcome.splat_count,
        labels,
    )
    .expect("write instance mask");
    let info = export_instance_assets(&project_root, || Ok(())).expect("export assets");
    println!(
        "      object.ply: {} splats; background.ply: {} splats; sum = {} (final = {})",
        info.object.splat_count,
        info.background.splat_count,
        info.object.splat_count + info.background.splat_count,
        outcome.splat_count
    );
    assert_eq!(
        info.object.splat_count + info.background.splat_count,
        outcome.splat_count,
        "split must be exhaustive"
    );
    assert_eq!(
        object_count, info.object.splat_count,
        "instance bitmap and object.ply must agree"
    );

    let mean = |select: &dyn Fn(usize) -> bool| -> (f64, usize) {
        let mut sum = 0.0_f64;
        let mut n = 0_usize;
        for (index, ratio) in outcome.ratios.iter().enumerate() {
            if select(index) {
                sum += *ratio as f64;
                n += 1;
            }
        }
        if n == 0 {
            (0.0, 0)
        } else {
            (sum / n as f64, n)
        }
    };
    let (object_mean, object_n) = mean(&|i| labels[i] != 0);
    let (background_mean, background_n) = mean(&|i| labels[i] == 0);
    let mut histogram = [0_usize; 11];
    for ratio in &outcome.ratios {
        let bucket = (*ratio * 10.0).round().clamp(0.0, 10.0) as usize;
        histogram[bucket] += 1;
    }
    println!("[stats] object points: {object_n} mean vote ratio = {object_mean:.4}");
    println!("[stats] background points: {background_n} mean vote ratio = {background_mean:.4}");
    println!("[stats] vote-ratio histogram (0.0..1.0, step 0.1):");
    for (bucket, count) in histogram.iter().enumerate() {
        println!("  {:.1}: {}", bucket as f64 / 10.0, count);
    }
    println!("OK");
}

/// Parses the three legacy positional arguments plus the optional filter flags
/// (`--min-cluster-points`, `--max-distance-ratio`; `none` disables the gate),
/// so an existing three-argument invocation keeps working with the defaults.
fn parse_arguments() -> (PathBuf, PathBuf, PathBuf, ClusterFilterConfig) {
    let mut positional: Vec<PathBuf> = Vec::new();
    let mut filter = ClusterFilterConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--min-cluster-points" => {
                let value = args.next().unwrap_or_else(|| fail("missing value"));
                filter.min_cluster_points = value
                    .parse()
                    .unwrap_or_else(|_| fail("--min-cluster-points needs a non-negative integer"));
            }
            "--max-distance-ratio" => {
                let value = args.next().unwrap_or_else(|| fail("missing value"));
                filter.max_distance_ratio =
                    if value.eq_ignore_ascii_case("none") {
                        None
                    } else {
                        Some(value.parse().unwrap_or_else(|_| {
                            fail("--max-distance-ratio needs a number or `none`")
                        }))
                    };
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other if other.starts_with("--") => fail(&format!("unknown option {other}")),
            _ => positional.push(PathBuf::from(argument)),
        }
    }
    if positional.len() != 3 {
        fail("expected exactly three positional arguments");
    }
    let masks_dir = positional.pop().expect("checked length");
    let model_dir = positional.pop().expect("checked length");
    let project_root = positional.pop().expect("checked length");
    (project_root, model_dir, masks_dir, filter)
}

fn fail(message: &str) -> ! {
    eprintln!("{message}\n{USAGE}");
    std::process::exit(2);
}

/// Prints the cluster size histogram (top 10) before and after filtering plus
/// the dropped cluster/point totals, so an acceptance run is self-documenting.
fn print_cluster_report(filtered: &ClusterFilterOutcome) {
    let total_clusters = filtered.clusters.len();
    let total_points = filtered.kept_points + filtered.dropped_points;
    println!(
        "      clusters: {} -> {} kept; splats {} -> {} kept, {} dropped in {} clusters",
        total_clusters,
        filtered.kept_clusters,
        total_points,
        filtered.kept_points,
        filtered.dropped_points,
        filtered.dropped_clusters
    );
    println!(
        "      scene diagonal (candidate AABB): {:.3}",
        filtered.scene_diagonal
    );
    println!("      cluster size histogram before filter (top 10):");
    for cluster in filtered.clusters.iter().take(10) {
        println!(
            "        label {:>3}: {:>6} splats  dist {:>7.3}  ratio {:>7.4}  {}",
            cluster.label,
            cluster.points,
            cluster.distance_to_main,
            cluster.distance_ratio,
            if cluster.kept { "keep" } else { "drop" }
        );
    }
    let kept: Vec<_> = filtered
        .clusters
        .iter()
        .filter(|cluster| cluster.kept)
        .collect();
    println!(
        "      cluster size histogram after filter (top 10 of {} kept):",
        kept.len()
    );
    for cluster in kept.iter().take(10) {
        println!(
            "        label {:>3}: {:>6} splats  dist {:>7.3}  ratio {:>7.4}",
            cluster.label, cluster.points, cluster.distance_to_main, cluster.distance_ratio
        );
    }
    if let Some(cluster) = filtered
        .clusters
        .iter()
        .filter(|cluster| !cluster.kept)
        .max_by_key(|cluster| cluster.points)
    {
        println!(
            "      largest dropped cluster: label {} with {} splats (dist {:.3}, ratio {:.4})",
            cluster.label, cluster.points, cluster.distance_to_main, cluster.distance_ratio
        );
    }
}
