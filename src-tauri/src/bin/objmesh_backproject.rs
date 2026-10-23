//! Example CLI: SAM2 mask back-projection voting + object/background split.
//!
//! Usage:
//!   objmesh_backproject <project_root> <colmap_model_dir> <masks_dir>
//!
//! Reads `<project_root>/final.ply` (read-only), votes every splat against
//! the per-frame mask PNGs in `<masks_dir>`, clusters the candidates and
//! writes `<project_root>/derived/instance.mask` plus
//! `<project_root>/assets/object.ply` / `background.ply`.
//! Prints vote-ratio statistics for acceptance evidence.

use std::path::PathBuf;

use ooo_splat::reconstruction::backproject::{
    label_object_clusters, load_frame_views, vote_splats, write_instance_mask, ClusterConfig,
    VoteConfig,
};
use ooo_splat::reconstruction::ply::stream_splat_cores;
use ooo_splat::reconstruction::splat_transform::export_instance_assets;

fn main() {
    let mut args = std::env::args().skip(1);
    let project_root = PathBuf::from(args.next().expect("usage: objmesh_backproject <project_root> <colmap_model_dir> <masks_dir>"));
    let model_dir = PathBuf::from(args.next().expect("missing <colmap_model_dir>"));
    let masks_dir = PathBuf::from(args.next().expect("missing <masks_dir>"));

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
    println!(
        "      splats: {}; candidates: {} ({:.2}%)",
        outcome.splat_count,
        outcome.candidates.iter().filter(|c| **c).count(),
        100.0 * outcome.candidates.iter().filter(|c| **c).count() as f64 / outcome.splat_count as f64
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
        &ClusterConfig { eps: 0.1, min_points: 10 },
    );
    println!("      clusters: {}", clusters.cluster_count);

    println!("[4/4] writing instance.mask and splitting assets");
    let object_count =
        write_instance_mask(&project_root.join("derived").join("instance.mask"), outcome.splat_count, &clusters.labels)
            .expect("write instance mask");
    let info = export_instance_assets(&project_root, || Ok(())).expect("export assets");
    println!(
        "      object.ply: {} splats; background.ply: {} splats; sum = {} (final = {})",
        info.object.splat_count,
        info.background.splat_count,
        info.object.splat_count + info.background.splat_count,
        outcome.splat_count
    );
    assert_eq!(info.object.splat_count + info.background.splat_count, outcome.splat_count, "split must be exhaustive");
    assert_eq!(object_count, info.object.splat_count, "instance bitmap and object.ply must agree");

    let mean = |select: &dyn Fn(usize) -> bool| -> (f64, usize) {
        let mut sum = 0.0_f64;
        let mut n = 0_usize;
        for (index, ratio) in outcome.ratios.iter().enumerate() {
            if select(index) {
                sum += *ratio as f64;
                n += 1;
            }
        }
        if n == 0 { (0.0, 0) } else { (sum / n as f64, n) }
    };
    let (object_mean, object_n) = mean(&|i| clusters.labels[i] != 0);
    let (background_mean, background_n) = mean(&|i| clusters.labels[i] == 0);
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
