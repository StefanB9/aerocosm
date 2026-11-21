//! `cargo xtask web build`: both wasm variants of the viewer plus the loader
//! page (plan D23).

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use tracing::info;

#[cfg(test)]
use crate::host::join_args;
pub(crate) use crate::host::{Host, SystemHost};
use crate::lock::locked_version;

/// The browser builds, in build order. Bevy selects WebGL2 at compile time, so
/// each is a separate binary, chosen at runtime by `web/index.html`.
const RENDERERS: [&str; 2] = ["webgpu", "webgl2"];
const WASM_TARGET: &str = "wasm32-unknown-unknown";
const VIEWER: &str = "aerocosm-viewer";

/// Binaryen optimisation flags for release builds. `wasm-bindgen` consumes
/// Rust's `target_features` custom section, so the features emitted by `rustc`
/// and `wasm-bindgen` must be enabled explicitly.
const WASM_OPT_FLAGS: [&str; 11] = [
    "-Os",
    "--strip-debug",
    "--strip-producers",
    "--enable-bulk-memory",
    "--enable-mutable-globals",
    "--enable-multivalue",
    "--enable-nontrapping-float-to-int",
    "--enable-reference-types",
    "--enable-sign-ext",
    "--enable-simd",
    "-o",
];

/// Normalises `path` to the host platform's native separator without altering
/// Windows prefixes (`C:\`, `\\server\share`, `\\?\`) or literal backslashes on
/// Unix.
fn normalise_path(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Cargo profile for the wasm builds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    /// Optimised with `wasm-release` and `wasm-opt`; what gets deployed.
    Release,
    /// Fast to build, large and slow to run.
    Dev,
}

impl Profile {
    const fn name(self) -> &'static str {
        match self {
            Self::Release => "wasm-release",
            Self::Dev => "dev",
        }
    }

    /// Cargo's output directory for the profile (`dev` builds land in `debug`).
    const fn artifact_dir(self) -> &'static str {
        match self {
            Self::Release => "wasm-release",
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
/// exactly) and, on release builds, that `wasm-opt` is available.
pub(crate) fn build(host: &mut impl Host, options: &BuildOptions) -> Result<()> {
    let lock = host.read_to_string(Path::new("Cargo.lock"))?;
    let expected = locked_version(&lock, "wasm-bindgen")
        .context("Cargo.lock has no wasm-bindgen entry")?
        .to_owned();
    let version_flag = [OsString::from("--version")];
    let printed = host
        .output("wasm-bindgen", &version_flag)
        .context("wasm-bindgen CLI not found; see README (Prerequisites)")?;
    let actual = bindgen_version(&printed)
        .with_context(|| format!("unexpected wasm-bindgen --version output {printed:?}"))?;
    if actual != expected {
        bail!(
            "wasm-bindgen CLI {actual} does not match crate {expected}; install it with `cargo \
             install wasm-bindgen-cli --version {expected} --locked`"
        );
    }
    if options.profile == Profile::Release {
        host.output("wasm-opt", &version_flag)
            .context("wasm-opt CLI not found; install Binaryen for release web builds")?;
    }

    let out = normalise_path(&options.out);
    let index_src = PathBuf::from_iter(["web", "index.html"]);
    let ground_src = PathBuf::from_iter(["assets", "ground"]);

    host.reset_dir(&out)?;
    host.copy(&index_src, &out.join("index.html"))?;
    // The ground layouts, fetched by the viewer at start (plan M5 D132), with
    // their licence.
    let ground_out = out.join(&ground_src);
    host.reset_dir(&ground_out)?;
    for name in host.list_dir(&ground_src)? {
        host.copy(&ground_src.join(&name), &ground_out.join(&name))?;
    }
    for renderer in RENDERERS {
        // Separate target directories keep both variants' build caches.
        let target_dir = PathBuf::from_iter(["target", "web-build", renderer]);
        let cargo_args = [
            OsString::from("build"),
            OsString::from("-p"),
            OsString::from(VIEWER),
            OsString::from("--target"),
            OsString::from(WASM_TARGET),
            OsString::from("--profile"),
            OsString::from(options.profile.name()),
            OsString::from("--no-default-features"),
            OsString::from("--features"),
            OsString::from(renderer),
            OsString::from("--target-dir"),
            target_dir.clone().into_os_string(),
        ];
        host.run(&options.cargo, &cargo_args)?;

        let wasm = target_dir
            .join(WASM_TARGET)
            .join(options.profile.artifact_dir())
            .join(format!("{VIEWER}.wasm"));
        let out_dir = out.join(renderer);
        let bindgen_args = [
            OsString::from("--target"),
            OsString::from("web"),
            OsString::from("--no-typescript"),
            OsString::from("--out-dir"),
            out_dir.clone().into_os_string(),
            wasm.into_os_string(),
        ];
        host.run("wasm-bindgen", &bindgen_args)?;

        if options.profile == Profile::Release {
            let bg_wasm = out_dir.join(format!("{VIEWER}_bg.wasm"));
            let mut opt_args = Vec::with_capacity(WASM_OPT_FLAGS.len() + 2);
            opt_args.extend(WASM_OPT_FLAGS.into_iter().map(OsString::from));
            opt_args.push(bg_wasm.clone().into_os_string());
            opt_args.push(bg_wasm.into_os_string());
            host.run("wasm-opt", &opt_args)?;
        }
    }
    info!(out = %out.display(), "browser build ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a platform-native path string from `/`-separated segments.
    fn native(slash_path: &str) -> String {
        slash_path
            .split('/')
            .collect::<PathBuf>()
            .display()
            .to_string()
    }

    /// Records every call; answers from canned values.
    #[derive(Default)]
    struct FakeHost {
        calls: Vec<String>,
        lock: String,
        bindgen_version: String,
        has_wasm_opt: bool,
    }

    impl Host for FakeHost {
        fn run(&mut self, program: &str, args: &[OsString]) -> Result<()> {
            self.calls
                .push(format!("run {program} {}", join_args(args)));
            Ok(())
        }

        fn output(&mut self, program: &str, args: &[OsString]) -> Result<String> {
            self.calls
                .push(format!("output {program} {}", join_args(args)));
            match program {
                "wasm-bindgen" => Ok(self.bindgen_version.clone()),
                "wasm-opt" if self.has_wasm_opt => Ok("wasm-opt version 123\n".to_owned()),
                "wasm-opt" => bail!("wasm-opt missing"),
                _ => bail!("unexpected program {program}"),
            }
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

        fn list_dir(&mut self, path: &Path) -> Result<Vec<String>> {
            self.calls.push(format!("list {}", path.display()));
            Ok(vec!["E.ground.bin".to_owned(), "LICENSE.txt".to_owned()])
        }

        fn append(&mut self, path: &Path, _text: &str) -> Result<()> {
            self.calls.push(format!("append {}", path.display()));
            Ok(())
        }

        fn machine(&mut self, os: &str) -> crate::bench::record::Machine {
            crate::bench::record::Machine::from_proc("", "", os)
        }

        fn has_display(&mut self) -> bool {
            false
        }
    }

    fn host(bindgen: &str) -> FakeHost {
        FakeHost {
            lock: "[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.130\"\n".to_owned(),
            bindgen_version: format!("wasm-bindgen {bindgen}\n"),
            has_wasm_opt: true,
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
        let flags = WASM_OPT_FLAGS.join(" ");
        let webgpu_bg = native("target/web/webgpu/aerocosm-viewer_bg.wasm");
        let webgl2_bg = native("target/web/webgl2/aerocosm-viewer_bg.wasm");
        let expected = [
            "read Cargo.lock".to_owned(),
            "output wasm-bindgen --version".to_owned(),
            "output wasm-opt --version".to_owned(),
            format!("reset {}", native("target/web")),
            format!(
                "copy {} {}",
                native("web/index.html"),
                native("target/web/index.html")
            ),
            format!("reset {}", native("target/web/assets/ground")),
            format!("list {}", native("assets/ground")),
            format!(
                "copy {} {}",
                native("assets/ground/E.ground.bin"),
                native("target/web/assets/ground/E.ground.bin")
            ),
            format!(
                "copy {} {}",
                native("assets/ground/LICENSE.txt"),
                native("target/web/assets/ground/LICENSE.txt")
            ),
            format!(
                "run cargo build -p aerocosm-viewer --target wasm32-unknown-unknown --profile \
                 wasm-release --no-default-features --features webgpu --target-dir {}",
                native("target/web-build/webgpu")
            ),
            format!(
                "run wasm-bindgen --target web --no-typescript --out-dir {} {}",
                native("target/web/webgpu"),
                native(
                    "target/web-build/webgpu/wasm32-unknown-unknown/wasm-release/aerocosm-viewer.\
                     wasm"
                )
            ),
            format!("run wasm-opt {flags} {webgpu_bg} {webgpu_bg}"),
            format!(
                "run cargo build -p aerocosm-viewer --target wasm32-unknown-unknown --profile \
                 wasm-release --no-default-features --features webgl2 --target-dir {}",
                native("target/web-build/webgl2")
            ),
            format!(
                "run wasm-bindgen --target web --no-typescript --out-dir {} {}",
                native("target/web/webgl2"),
                native(
                    "target/web-build/webgl2/wasm32-unknown-unknown/wasm-release/aerocosm-viewer.\
                     wasm"
                )
            ),
            format!("run wasm-opt {flags} {webgl2_bg} {webgl2_bg}"),
        ];
        assert_eq!(host.calls, expected);
        Ok(())
    }

    #[test]
    fn test_build_dev_profile_uses_debug_artifacts_and_skips_wasm_opt() -> Result<()> {
        let mut host = host("0.2.130");
        host.has_wasm_opt = false;
        build(&mut host, &options(Profile::Dev)).context("build")?;
        assert!(host.calls.iter().any(|c| c.contains("--profile dev")));
        let debug_wasm = native("wasm32-unknown-unknown/debug/aerocosm-viewer.wasm");
        assert!(host.calls.iter().any(|c| c.contains(&debug_wasm)));
        assert!(host.calls.iter().all(|c| !c.contains("wasm-opt")));
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
    fn test_build_refuses_missing_wasm_opt_before_building() -> Result<()> {
        let mut host = host("0.2.130");
        host.has_wasm_opt = false;
        let error = build(&mut host, &options(Profile::Release))
            .err()
            .context("must fail")?;
        let message = format!("{error:#}");
        assert!(message.contains("wasm-opt"), "{message}");
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

    #[test]
    fn test_normalise_path_uses_platform_components() {
        assert_eq!(
            normalise_path(Path::new("target/web")),
            PathBuf::from_iter(["target", "web"])
        );
    }
}
