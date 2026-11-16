//! The viewer's frames without a window or GPU (plan M5 D140): CPU work per
//! frame at fixed views, printed as `bench <name> <value> <unit>` lines.
//!
//! `cargo bench -p aerocosm-viewer --bench frames --profile bench-unwind --
//! [frames]`; the default is 300 frames per view after 60 to settle.

use std::{io::Write as _, process::ExitCode, time::Duration};

use aerocosm_viewer::bench::{Headless, View, summary};
use anyhow::{Context, Result};

/// Frames run before measuring: grounds in view build over a few frames.
const SETTLE: usize = 60;

fn measure(view: View, frames: usize, out: &mut impl std::io::Write) -> Result<()> {
    let mut viewer = Headless::new(view).with_context(|| view.name())?;
    for _ in 0..SETTLE {
        viewer.frame();
    }
    let mut samples: Vec<f64> = (0..frames).map(|_| ms(viewer.frame())).collect();
    let name = view.name();
    if let Some(s) = summary(&mut samples) {
        for (figure, value) in [
            ("median", s[0]),
            ("p95", s[1]),
            ("max", s[2]),
            ("mean", s[3]),
        ] {
            writeln!(out, "bench viewer/{name}/frame_{figure} {value:.6} ms")?;
        }
    }
    writeln!(
        out,
        "bench viewer/{name}/entities {} count",
        viewer.entities()
    )?;
    Ok(())
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

fn main() -> ExitCode {
    // Cargo passes `--bench`; a number is the frames to measure.
    let frames = std::env::args()
        .skip(1)
        .find_map(|arg| arg.parse().ok())
        .unwrap_or(300);
    let mut out = std::io::stdout().lock();
    for view in View::ALL {
        if let Err(error) = measure(view, frames, &mut out) {
            let _ = writeln!(std::io::stderr(), "viewer frames: {error:#}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
