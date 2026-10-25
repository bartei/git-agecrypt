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
