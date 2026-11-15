//! The machine's processes and files, behind a trait so that xtask's logic
//! is tested with fakes.

use std::{fs, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use tracing::info;

/// Side effects of a task (processes and files), so that task logic can be
/// tested with a fake.
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
    /// The names of the files in a directory, sorted.
    fn list_dir(&mut self, path: &Path) -> Result<Vec<String>>;
    /// Appends `text` to a file, creating it if needed.
    fn append(&mut self, path: &Path, text: &str) -> Result<()>;
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

    fn list_dir(&mut self, path: &Path) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in
            fs::read_dir(path).with_context(|| format!("cannot list {}", path.display()))?
        {
            let entry = entry.with_context(|| format!("cannot list {}", path.display()))?;
            if entry.file_type().is_ok_and(|kind| kind.is_file())
                && let Ok(name) = entry.file_name().into_string()
            {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }

    fn append(&mut self, path: &Path, text: &str) -> Result<()> {
        use std::io::Write as _;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", path.display()))
    }
}
