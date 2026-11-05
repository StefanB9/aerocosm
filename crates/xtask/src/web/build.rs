//! `cargo xtask web build`: both wasm variants of the viewer plus the loader
//! page (plan D23).

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::lock::locked_version;

/// The browser builds, in build order. Bevy selects WebGL2 at compile time, so
/// each is a separate binary, chosen at runtime by `web/index.html`.
const RENDERERS: [&str; 2] = ["webgpu", "webgl2"];
const WASM_TARGET: &str = "wasm32-unknown-unknown";
const VIEWER: &str = "aerocosm-viewer";

/// Side effects of a build, so the build logic can be tested with a fake.
pub(crate) trait Host {
    /// Runs a program to completion, failing on a non-zero exit.
    fn run(&mut self, program: &str, args: &[String]) -> Result<()>;
    /// Runs a program and returns its standard output.
    fn output(&mut self, program: &str, args: &[String]) -> Result<String>;
    /// Reads a text file.
    fn read_to_string(&mut self, path: &Path) -> Result<String>;
    /// Removes a directory if present and creates it empty.
    fn reset_dir(&mut self, path: &Path) -> Result<()>;
    /// Copies a file.
    fn copy(&mut self, from: &Path, to: &Path) -> Result<()>;
}

/// The real machine.
pub(crate) struct SystemHost;

impl Host for SystemHost {
    fn run(&mut self, program: &str, args: &[String]) -> Result<()> {
        info!(program, args = %args.join(" "), "running");
        let status = Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("cannot start {program}"))?;
        if !status.success() {
            bail!("{program} failed with {status}");
        }
        Ok(())
    }

    fn output(&mut self, program: &str, args: &[String]) -> Result<String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("cannot start {program}"))?;
        if !output.status.success() {
            bail!("{program} failed with {}", output.status);
        }
        String::from_utf8(output.stdout).with_context(|| format!("{program} printed non-UTF-8"))
    }

    fn read_to_string(&mut self, path: &Path) -> Result<String> {
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
    }

    fn reset_dir(&mut self, path: &Path) -> Result<()> {
        if path.exists() {
            fs::remove_dir_all(path)
                .with_context(|| format!("cannot remove {}", path.display()))?;
        }
        fs::create_dir_all(path).with_context(|| format!("cannot create {}", path.display()))
    }

    fn copy(&mut self, from: &Path, to: &Path) -> Result<()> {
        fs::copy(from, to)
            .map(drop)
            .with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))
    }
}

/// Cargo profile for the wasm builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    /// Optimised; what gets deployed.
    Release,
    /// Fast to build, large and slow to run.
    Dev,
}

impl Profile {
    const fn name(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Dev => "dev",
        }
    }

    /// Cargo's output directory for the profile (`dev` builds land in `debug`).
    const fn artifact_dir(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Dev => "debug",
        }
    }
}

/// What to build and where, relative to the workspace root.
#[derive(Debug, Clone)]
pub(crate) struct BuildOptions {
    pub(crate) out: PathBuf,
    pub(crate) profile: Profile,
    /// The cargo executable (the one running xtask, when known).
    pub(crate) cargo: String,
}

/// The version from `wasm-bindgen --version` output such as `wasm-bindgen
/// 0.2.130`.
fn bindgen_version(output: &str) -> Option<&str> {
    let mut words = output.split_whitespace();
    (words.next() == Some("wasm-bindgen"))
        .then(|| words.next())
        .flatten()
}

/// Builds every renderer variant into `options.out`, after checking that the
/// `wasm-bindgen` CLI matches the crate locked in `Cargo.lock` (they must agree
/// exactly).
pub(crate) fn build(host: &mut impl Host, options: &BuildOptions) -> Result<()> {
    let lock = host.read_to_string(Path::new("Cargo.lock"))?;
    let expected = locked_version(&lock, "wasm-bindgen")
        .context("Cargo.lock has no wasm-bindgen entry")?
        .to_owned();
    let printed = host
        .output("wasm-bindgen", &["--version".to_owned()])
        .context("wasm-bindgen CLI not found; see README (Prerequisites)")?;
    let actual = bindgen_version(&printed)
        .with_context(|| format!("unexpected wasm-bindgen --version output {printed:?}"))?;
    if actual != expected {
        bail!(
            "wasm-bindgen CLI {actual} does not match crate {expected}; install it with `cargo \
             install wasm-bindgen-cli --version {expected} --locked`"
        );
    }

    host.reset_dir(&options.out)?;
    host.copy(Path::new("web/index.html"), &options.out.join("index.html"))?;
    for renderer in RENDERERS {
        // Separate target directories keep both variants' build caches.
        let target_dir = Path::new("target/web-build").join(renderer);
        let to_string = |path: &Path| path.display().to_string();
        let cargo_args = [
            "build",
            "-p",
            VIEWER,
            "--target",
            WASM_TARGET,
            "--profile",
            options.profile.name(),
            "--no-default-features",
            "--features",
            renderer,
            "--target-dir",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([to_string(&target_dir)])
        .collect::<Vec<_>>();
        host.run(&options.cargo, &cargo_args)?;

        let wasm = target_dir
            .join(WASM_TARGET)
            .join(options.profile.artifact_dir())
            .join(format!("{VIEWER}.wasm"));
        let bindgen_args = ["--target", "web", "--no-typescript", "--out-dir"]
            .into_iter()
            .map(str::to_owned)
            .chain([to_string(&options.out.join(renderer)), to_string(&wasm)])
            .collect::<Vec<_>>();
        host.run("wasm-bindgen", &bindgen_args)?;
    }
    info!(out = %options.out.display(), "browser build ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records every call; answers from canned values.
    #[derive(Default)]
    struct FakeHost {
        calls: Vec<String>,
        lock: String,
        bindgen_version: String,
    }

    impl Host for FakeHost {
        fn run(&mut self, program: &str, args: &[String]) -> Result<()> {
            self.calls.push(format!("run {program} {}", args.join(" ")));
            Ok(())
        }

        fn output(&mut self, program: &str, args: &[String]) -> Result<String> {
            self.calls
                .push(format!("output {program} {}", args.join(" ")));
            Ok(self.bindgen_version.clone())
        }

        fn read_to_string(&mut self, path: &Path) -> Result<String> {
            self.calls.push(format!("read {}", path.display()));
            if path == Path::new("Cargo.lock") {
                Ok(self.lock.clone())
            } else {
                bail!("unexpected read of {}", path.display())
            }
        }

        fn reset_dir(&mut self, path: &Path) -> Result<()> {
            self.calls.push(format!("reset {}", path.display()));
            Ok(())
        }

        fn copy(&mut self, from: &Path, to: &Path) -> Result<()> {
            self.calls
                .push(format!("copy {} {}", from.display(), to.display()));
            Ok(())
        }
    }

    fn host(bindgen: &str) -> FakeHost {
        FakeHost {
            lock: "[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.130\"\n".to_owned(),
            bindgen_version: format!("wasm-bindgen {bindgen}\n"),
            ..FakeHost::default()
        }
    }

    fn options(profile: Profile) -> BuildOptions {
        BuildOptions {
            out: PathBuf::from("target/web"),
            profile,
            cargo: "cargo".to_owned(),
        }
    }

    #[test]
    fn test_build_runs_both_variants_in_order() -> Result<()> {
        let mut host = host("0.2.130");
        build(&mut host, &options(Profile::Release)).context("build")?;
        let expected = [
            "read Cargo.lock",
            "output wasm-bindgen --version",
            "reset target/web",
            "copy web/index.html target/web/index.html",
            "run cargo build -p aerocosm-viewer --target wasm32-unknown-unknown --profile release \
             --no-default-features --features webgpu --target-dir target/web-build/webgpu",
            "run wasm-bindgen --target web --no-typescript --out-dir target/web/webgpu \
             target/web-build/webgpu/wasm32-unknown-unknown/release/aerocosm-viewer.wasm",
            "run cargo build -p aerocosm-viewer --target wasm32-unknown-unknown --profile release \
             --no-default-features --features webgl2 --target-dir target/web-build/webgl2",
            "run wasm-bindgen --target web --no-typescript --out-dir target/web/webgl2 \
             target/web-build/webgl2/wasm32-unknown-unknown/release/aerocosm-viewer.wasm",
        ];
        assert_eq!(host.calls, expected);
        Ok(())
    }

    #[test]
    fn test_build_dev_profile_uses_debug_artifacts() -> Result<()> {
        let mut host = host("0.2.130");
        build(&mut host, &options(Profile::Dev)).context("build")?;
        assert!(host.calls.iter().any(|c| c.contains("--profile dev")));
        assert!(
            host.calls
                .iter()
                .any(|c| c.contains("/wasm32-unknown-unknown/debug/aerocosm-viewer.wasm"))
        );
        Ok(())
    }

    #[test]
    fn test_build_refuses_mismatched_wasm_bindgen_before_building() -> Result<()> {
        let mut host = host("0.2.129");
        let error = build(&mut host, &options(Profile::Release))
            .err()
            .context("must fail")?;
        let message = format!("{error:#}");
        assert!(
            message.contains("0.2.129") && message.contains("0.2.130"),
            "{message}"
        );
        assert!(
            host.calls.iter().all(|c| !c.starts_with("run ")),
            "nothing built: {:?}",
            host.calls
        );
        Ok(())
    }

    #[test]
    fn test_bindgen_version_parsing() {
        assert_eq!(bindgen_version("wasm-bindgen 0.2.130\n"), Some("0.2.130"));
        assert_eq!(bindgen_version("something else"), None);
    }
}
