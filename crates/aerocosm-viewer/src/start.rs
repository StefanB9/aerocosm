//! Where the viewer opens: the whole world (plan M5 D133), or a view given on
//! the native command line as `--view <lat>,<lon>,<metres per pixel>`, for
//! looking at one airport or taking screenshots.

use std::time::Duration;

use aerocosm_core::LatLon;
use anyhow::{Context, Result, bail};
use bevy::ecs::resource::Resource;

/// Centre of the opening world view: Europe and Africa in the middle, the
/// Americas and Asia either side.
const HOME: (f64, f64) = (25.0, 10.0);

/// How long the measured scenes settle and run (plan M9 D182).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Measure {
    /// Time given to each scene before frames count: the ground loads, the
    /// pool fills.
    pub(crate) settle: Duration,
    /// Time each scene is measured.
    pub(crate) span: Duration,
}

impl Measure {
    /// A run for the record.
    pub(crate) const FULL: Self = Self {
        settle: Duration::from_secs(5),
        span: Duration::from_secs(10),
    };
    /// A run that only checks everything works.
    pub(crate) const QUICK: Self = Self {
        settle: Duration::from_secs(2),
        span: Duration::from_secs(1),
    };
}

/// What the command line asks for: where to open, and whether to measure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Launch {
    pub(crate) view: StartView,
    pub(crate) measure: Option<Measure>,
}

impl Launch {
    /// The launch the command-line `args` (without the program name) ask
    /// for: `--measure [quick]`, `--view <lat>,<lon>,<metres per pixel>` or
    /// nothing.
    pub(crate) fn from_args(args: impl Iterator<Item = String>) -> Result<Self> {
        let args: Vec<String> = args.collect();
        if args.first().map(String::as_str) != Some("--measure") {
            return Ok(Self {
                view: StartView::from_args(args.into_iter())?,
                measure: None,
            });
        }
        let measure = match args.get(1).map(String::as_str) {
            None => Measure::FULL,
            Some("quick") => Measure::QUICK,
            Some(other) => bail!("--measure {other}: expected nothing or quick"),
        };
        if let Some(extra) = args.get(2) {
            bail!("unexpected argument {extra}");
        }
        Ok(Self {
            view: StartView::home()?,
            measure: Some(measure),
        })
    }
}

/// The opening view.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub(crate) struct StartView {
    pub(crate) centre: LatLon,
    /// Map units per pixel; the whole world when absent.
    pub(crate) scale: Option<f32>,
}

impl StartView {
    /// The whole world.
    pub(crate) fn home() -> Result<Self> {
        Ok(Self {
            centre: LatLon::new(HOME.0, HOME.1)?,
            scale: None,
        })
    }

    /// The view the command-line `args` (without the program name) ask for,
    /// else the whole world.
    pub(crate) fn from_args(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let Some(flag) = args.next() else {
            return Self::home();
        };
        if flag != "--view" {
            bail!("unknown argument {flag} (expected --view <lat>,<lon>,<metres per pixel>)");
        }
        let value = args
            .next()
            .context("--view needs <lat>,<lon>,<metres per pixel>")?;
        let parts: Vec<&str> = value.split(',').collect();
        let [lat, lon, scale] = parts.as_slice() else {
            bail!("--view {value}: expected <lat>,<lon>,<metres per pixel>");
        };
        let scale: f32 = scale
            .trim()
            .parse()
            .with_context(|| format!("scale {scale}"))?;
        if !(scale.is_finite() && scale > 0.0) {
            bail!("--view {value}: the scale must be positive");
        }
        if let Some(extra) = args.next() {
            bail!("unexpected argument {extra}");
        }
        Ok(Self {
            centre: LatLon::new(
                lat.trim()
                    .parse()
                    .with_context(|| format!("latitude {lat}"))?,
                lon.trim()
                    .parse()
                    .with_context(|| format!("longitude {lon}"))?,
            )?,
            scale: Some(scale),
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn test_launch_reads_measure_and_view() -> Result<()> {
        assert_eq!(
            Launch::from_args(args(&[]))?,
            Launch {
                view: StartView::home()?,
                measure: None,
            }
        );
        let full = Launch::from_args(args(&["--measure"]))?;
        assert_eq!(full.view, StartView::home()?);
        assert_eq!(full.measure, Some(Measure::FULL));
        assert_eq!(
            Launch::from_args(args(&["--measure", "quick"]))?.measure,
            Some(Measure::QUICK)
        );
        let view = Launch::from_args(args(&["--view", "50.03,8.57,0.8"]))?;
        assert_eq!(view.measure, None);
        assert_eq!(view.view.scale, Some(0.8));
        assert!(Launch::from_args(args(&["--measure", "slow"])).is_err());
        assert!(Launch::from_args(args(&["--measure", "quick", "--view", "1,2,3"])).is_err());
        Ok(())
    }

    #[test]
    fn test_no_arguments_open_the_world() -> Result<()> {
        assert_eq!(StartView::from_args(args(&[]))?, StartView::home()?);
        Ok(())
    }

    #[test]
    fn test_view_argument_centres_and_zooms() -> Result<()> {
        let view = StartView::from_args(args(&["--view", "50.03, 8.57,0.8"]))?;
        assert!((view.centre.latitude().value() - 50.03).abs() < 1e-9);
        assert!((view.centre.longitude().value() - 8.57).abs() < 1e-9);
        assert_eq!(view.scale, Some(0.8));
        Ok(())
    }

    #[test]
    fn test_bad_view_arguments_are_rejected() {
        for bad in [
            &["--zoom", "1"][..],
            &["--view"],
            &["--view", "50,8"],
            &["--view", "50,8,0"],
            &["--view", "95,8,1"],
            &["--view", "50,x,1"],
            &["--view", "50,8,1", "more"],
        ] {
            assert!(StartView::from_args(args(bad)).is_err(), "{bad:?}");
        }
    }
}
