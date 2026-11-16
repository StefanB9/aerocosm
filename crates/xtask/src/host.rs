//! The machine's processes and files, behind a trait so that xtask's logic
//! is tested with fakes.

use std::{ffi::OsString, fs, path::Path, process::Command};

use anyhow::{Context, Result, bail};
use tracing::info;

/// Side effects of a task (processes and files), so that task logic can be
/// tested with a fake.
pub(crate) trait Host {
    /// Runs a program to completion, failing on a non-zero exit.
    fn run(&mut self, program: &str, args: &[OsString]) -> Result<()>;
    /// Runs a program and returns its standard output.
    fn output(&mut self, program: &str, args: &[OsString]) -> Result<String>;
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

/// Formats command-line arguments for human-readable logging.
pub(crate) fn join_args(args: &[OsString]) -> String {
    let mut rendered = String::new();
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            rendered.push(' ');
        }
        rendered.push_str(&arg.to_string_lossy());
    }
    rendered
}

/// The real machine.
pub(crate) struct SystemHost;

impl Host for SystemHost {
    fn run(&mut self, program: &str, args: &[OsString]) -> Result<()> {
        info!(program, args = %join_args(args), "running");
        let status = Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("cannot start {program}"))?;
        if !status.success() {
            bail!("{program} failed with {status}");
        }
        Ok(())
    }

    fn output(&mut self, program: &str, args: &[OsString]) -> Result<String> {
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
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("cannot write {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_append_creates_the_file_and_its_directory() -> Result<()> {
        let root = std::env::temp_dir().join(format!("xtask-append-{}", std::process::id()));
        let file = root.join("benchmarks/history.jsonl");
        SystemHost.append(&file, "one\n")?;
        SystemHost.append(&file, "two\n")?;
        assert_eq!(SystemHost.read_to_string(&file)?, "one\ntwo\n");
        fs::remove_dir_all(&root)?;
        Ok(())
    }
}
