//! Micro-benchmarks of the viewer's own logic (plan M5 D139): basemap
//! decoding, label placement over Europe, reprojection of every label
//! source, the ground streaming plan and tessellation.
//!
//! `cargo bench -p aerocosm-viewer --bench micro --profile bench-unwind`.

use std::{hint::black_box, io::Write as _, process::ExitCode};

use aerocosm_viewer::bench::Micro;
use anyhow::Result;
use criterion::Criterion;

fn run(c: &mut Criterion) -> Result<()> {
    let mut micro = Micro::new()?;
    let mut group = c.benchmark_group("viewer");
    group.sample_size(20);
    group.bench_function("decode_basemap", |b| {
        b.iter(|| black_box(Micro::decode_basemap()));
    });
    group.finish();
    c.bench_function("viewer/place_labels_europe", |b| {
        b.iter(|| black_box(micro.place_labels()));
    });
    c.bench_function("viewer/reproject_labels", |b| {
        b.iter(|| black_box(micro.reproject()));
    });
    c.bench_function("viewer/stream_plan_frankfurt", |b| {
        b.iter(|| black_box(micro.stream_plan()));
    });
    c.bench_function("viewer/tessellate_country", |b| {
        b.iter(|| black_box(micro.tessellate_country()));
    });
    c.bench_function("viewer/tessellate_frankfurt", |b| {
        b.iter(|| black_box(micro.tessellate_frankfurt()));
    });
    Ok(())
}

fn main() -> ExitCode {
    let mut criterion = Criterion::default().configure_from_args();
    if let Err(error) = run(&mut criterion) {
        let _ = writeln!(std::io::stderr(), "viewer micro-benchmarks: {error:#}");
        return ExitCode::FAILURE;
    }
    criterion.final_summary();
    ExitCode::SUCCESS
}
