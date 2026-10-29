// SPDX-License-Identifier: MPL-2.0

//! Thin helpers over the git CLI for what libgit2 does not offer here:
//! check-attr in bulk and pathspecs through the filter (rekey). Paths never
//! go on argv: a repo can hold thousands of encrypted files.

use std::{
    collections::HashSet,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

use anyhow::{Context as _, Result, bail};

pub(crate) fn output(dir: &Path, args: &[&str]) -> Result<Output> {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .with_context(|| format!("cannot run git {args:?}"))
}

fn checked(out: Output, args: &[&str]) -> Result<Vec<u8>> {
    if !out.status.success() {
        bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// Run git with `input` on stdin, fed from a thread so large inputs and large
/// outputs cannot deadlock on full pipes.
fn run_with_input(dir: &Path, args: &[&str], input: Vec<u8>) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run git {args:?}"))?;
    let mut stdin = child.stdin.take().context("no stdin for git")?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output()?;
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("git stdin writer panicked"))??;
    checked(out, args)
}

fn nul_joined(paths: &[String]) -> Vec<u8> {
    let mut input = paths.join("\0").into_bytes();
    input.push(0);
    input
}

/// Run a git command that takes pathspecs (checkout, add, ...) on `paths`,
/// literally and through stdin.
pub(crate) fn run_with_paths(dir: &Path, args: &[&str], paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut full = vec!["--literal-pathspecs"];
    full.extend(args);
    full.extend(["--pathspec-from-file=-", "--pathspec-file-nul"]);
    run_with_input(dir, &full, nul_joined(paths))?;
    Ok(())
}

fn nul_split(bytes: &[u8]) -> Result<Vec<String>> {
    bytes
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| Ok(String::from_utf8(s.to_vec())?))
        .collect()
}

/// Tracked files under `pathspecs` (all when empty), as paths from the root.
pub(crate) fn tracked(root: &Path, pathspecs: &[&str]) -> Result<Vec<String>> {
    let mut args = vec!["ls-files", "-z", "--full-name", "--"];
    args.extend(pathspecs);
    nul_split(&checked(output(root, &args)?, &args)?)
}

/// Subset of `paths` whose filter attribute is git-agecrypt, in one git call.
pub(crate) fn filtered(root: &Path, paths: &[String]) -> Result<HashSet<String>> {
    if paths.is_empty() {
        return Ok(HashSet::new());
    }
    let out = run_with_input(
        root,
        &["check-attr", "--stdin", "-z", "filter"],
        nul_joined(paths),
    )?;
    Ok(nul_split(&out)?
        .chunks(3)
        .filter(|t| t.len() == 3 && t[2] == "git-agecrypt")
        .map(|t| t[0].clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            st.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&st.stderr)
        );
    }

    /// A repo with two tracked files, one under the git-agecrypt filter.
    fn repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q", "--initial-branch=main"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "T"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::create_dir_all(dir.path().join("secrets/deep")).unwrap();
        std::fs::write(dir.path().join("secrets/deep/s.txt"), "s").unwrap();
        std::fs::write(dir.path().join("plain.txt"), "p").unwrap();
        std::fs::write(
            dir.path().join(".gitattributes"),
            "secrets/** filter=git-agecrypt diff=git-agecrypt -text\n",
        )
        .unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "init"]);
        dir
    }

    #[test]
    fn tracked_lists_everything_or_the_given_pathspecs_from_the_root() {
        let dir = repo();
        let mut all = tracked(dir.path(), &[]).unwrap();
        all.sort();
        assert_eq!(
            all,
            vec![".gitattributes", "plain.txt", "secrets/deep/s.txt"]
        );
        assert_eq!(
            tracked(dir.path(), &["secrets"]).unwrap(),
            vec!["secrets/deep/s.txt"]
        );
        // Pathspecs are relative to the directory git runs in; the output is
        // still relative to the root, which is what `rekey` needs.
        assert_eq!(
            tracked(&dir.path().join("secrets/deep"), &["s.txt"]).unwrap(),
            vec!["secrets/deep/s.txt"]
        );
        assert!(tracked(dir.path(), &["nothing-here"]).unwrap().is_empty());
    }

    #[test]
    fn filtered_keeps_only_paths_under_the_filter() {
        let dir = repo();
        let paths: Vec<String> = ["plain.txt", "secrets/deep/s.txt", ".gitattributes"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let kept = filtered(dir.path(), &paths).unwrap();
        assert_eq!(
            kept.into_iter().collect::<Vec<_>>(),
            vec!["secrets/deep/s.txt"]
        );
        assert!(filtered(dir.path(), &[]).unwrap().is_empty());
    }

    #[test]
    fn run_with_paths_feeds_pathspecs_on_stdin_and_skips_empty_lists() {
        let dir = repo();
        run_with_paths(dir.path(), &["add", "--renormalize"], &[]).unwrap();
        std::fs::write(dir.path().join("plain.txt"), "changed").unwrap();
        run_with_paths(dir.path(), &["add"], &["plain.txt".to_string()]).unwrap();
        let out = output(dir.path(), &["diff", "--cached", "--name-only"]).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "plain.txt");
    }

    #[test]
    fn git_failures_carry_the_command_and_its_stderr() {
        let dir = repo();
        let err =
            run_with_paths(dir.path(), &["no-such-subcommand"], &["x".to_string()]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no-such-subcommand"), "{msg}");
        // Not a repository at all, for both the plain and the stdin-fed paths.
        let empty = TempDir::new().unwrap();
        let err = tracked(empty.path(), &[]).unwrap_err();
        assert!(format!("{err:#}").contains("ls-files"), "{err:#}");
        let err = filtered(empty.path(), &["x".to_string()]).unwrap_err();
        assert!(format!("{err:#}").contains("check-attr"), "{err:#}");
    }

    #[test]
    fn nul_split_rejects_non_utf8_paths() {
        assert!(nul_split(b"ok\0\xff\xfe\0").is_err());
        assert_eq!(nul_split(b"a\0b\0\0").unwrap(), vec!["a", "b"]);
    }
}
