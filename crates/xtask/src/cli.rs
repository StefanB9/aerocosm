//! Command-line interface of `cargo xtask`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

/// Project tooling for aerocosm.
#[derive(Debug, Parser)]
#[command(name = "cargo xtask", about)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Task groups.
#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// The browser viewer.
    #[command(subcommand)]
    Web(WebCommand),
}

/// `cargo xtask web …`
#[derive(Debug, Subcommand)]
pub(crate) enum WebCommand {
    /// Build the WebGPU and WebGL2 variants and the loader page.
    Build(BuildArgs),
    /// Serve a browser build on localhost.
    Serve(ServeArgs),
}

/// Options for `web build`.
#[derive(Debug, Args)]
pub(crate) struct BuildArgs {
    /// Output directory, relative to the workspace root.
    #[arg(long, default_value = "target/web")]
    pub(crate) out: PathBuf,
    /// Use the dev profile: builds faster, much larger and slower output.
    #[arg(long)]
    pub(crate) dev: bool,
}

/// Options for `web serve`.
#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    /// Directory to serve, relative to the workspace root.
    #[arg(long, default_value = "target/web")]
    pub(crate) dir: PathBuf,
    /// Port on 127.0.0.1.
    #[arg(long, default_value_t = 8080)]
    pub(crate) port: u16,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::{Context, Result, bail};

    use super::*;

    #[test]
    fn test_cli_web_build_defaults_and_flags() -> Result<()> {
        let Command::Web(WebCommand::Build(args)) = Cli::try_parse_from(["xtask", "web", "build"])
            .context("parse")?
            .command
        else {
            bail!("expected web build");
        };
        assert_eq!(args.out, Path::new("target/web"));
        assert!(!args.dev);
        let Command::Web(WebCommand::Build(args)) =
            Cli::try_parse_from(["xtask", "web", "build", "--dev", "--out", "x"])
                .context("parse")?
                .command
        else {
            bail!("expected web build");
        };
        assert!(args.dev);
        assert_eq!(args.out, Path::new("x"));
        Ok(())
    }

    #[test]
    fn test_cli_web_serve_defaults_and_port() -> Result<()> {
        let Command::Web(WebCommand::Serve(args)) =
            Cli::try_parse_from(["xtask", "web", "serve", "--port", "9000"])
                .context("parse")?
                .command
        else {
            bail!("expected web serve");
        };
        assert_eq!(args.port, 9000);
        assert_eq!(args.dir, Path::new("target/web"));
        Ok(())
    }

    #[test]
    fn test_cli_rejects_unknown_tasks() {
        assert!(Cli::try_parse_from(["xtask", "web", "deploy"]).is_err());
        assert!(Cli::try_parse_from(["xtask"]).is_err());
    }
}
