use std::path::PathBuf;

use crate::{Result, git};

use crate::config::Validated;
use crate::git::Repository;
use crate::{config::AgeIdentity, ctx::Context};

pub(crate) struct CommandContext<C: Context> {
    ctx: C,
}

impl<C: Context> CommandContext<C> {
    pub fn new(ctx: C) -> Self {
        Self { ctx }
    }

    pub(crate) fn init(&self) -> Result<()> {
        let exe = shell_quote(&self.ctx.current_exe()?);
        let repo = self.ctx.repo();
        ensure_state(repo.set_config("filter.git-agecrypt.required", "true"))?;
        ensure_state(
            repo.set_config("filter.git-agecrypt.smudge", &format!("{exe} smudge -f %f")),
        )?;
        ensure_state(repo.set_config("filter.git-agecrypt.clean", &format!("{exe} clean -f %f")))?;
        ensure_state(repo.set_config("diff.git-agecrypt.textconv", &format!("{exe} textconv")))?;
        Ok(())
    }

    pub(crate) fn deinit(&self) -> Result<()> {
        let repo = self.ctx.repo();
        ensure_state(repo.remove_config_section("filter.git-agecrypt"))?;
        ensure_state(repo.remove_config_section("diff.git-agecrypt"))?;

        self.ctx.remove_sidecar_files()?;
        Ok(())
    }

    pub(crate) fn list_identities(&self) -> Result<()> {
        self.print_identities()
    }

    pub(crate) fn status(&self) -> Result<()> {
        self.list_identities()?;
        println!();
        self.list_recipients()?;
        Ok(())
    }

    pub(crate) fn add_identity(&self, identity: PathBuf) -> Result<()> {
        self.ctx
            .age_identities()
            .add(AgeIdentity::try_from(identity)?)?;
        Ok(())
    }

    pub(crate) fn remove_identity(&self, identity: PathBuf) -> Result<()> {
        self.ctx
            .age_identities()
            .remove(AgeIdentity::try_from(identity)?)?;
        Ok(())
    }

    fn print_identities(&self) -> Result<()> {
        let identities = self.ctx.age_identities().list()?;

        let padding = identities.iter().map(|i| i.path.len()).max().unwrap_or(0);
        println!("The following identities are currently configured:");
        for i in &identities {
            if let Err(err) = i.validate() {
                println!("    ⨯ {:padding$} -- {:?}", i.path, err, padding = padding);
            } else {
                println!("    ✓ {}", i.path);
            }
        }
        Ok(())
    }

    pub fn add_recipients(&self, recipients: Vec<String>, paths: Vec<PathBuf>) -> Result<()> {
        let mut cfg = self.ctx.config()?;

        cfg.add(recipients, paths)?;

        cfg.save()?;
        Ok(())
    }

    pub fn remove_recipients(&self, recipients: Vec<String>, paths: Vec<PathBuf>) -> Result<()> {
        let mut cfg = self.ctx.config()?;
        cfg.remove(recipients, paths)?;
        cfg.save()?;
        Ok(())
    }

    pub fn list_recipients(&self) -> Result<()> {
        let cfg = self.ctx.config()?;
        let recipients = cfg.list();

        println!("The following recipients are configured:");
        for (p, r) in recipients {
            println!("    {p}: {r}");
        }
        Ok(())
    }
}
fn ensure_state(result: git::Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(err) => match err {
            git::Error::AlreadyExists(_) => Ok(()),
            git::Error::NotExist(_) => Ok(()),
            err => Err(anyhow::anyhow!(err)),
        },
    }
}

/// Wrap a path for embedding into a `git config filter.*` command line.
///
/// Git invokes filter commands through `sh -c`, so the stored string is
/// re-parsed by a POSIX shell (msys2 sh on Windows). Plain double quotes
/// handle spaces; we additionally backslash-escape `\` and `"` so paths
/// containing those characters round-trip safely.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::shell_quote;

    #[test]
    fn quote_wraps_simple_path() {
        assert_eq!(
            shell_quote("/usr/local/bin/git-agecrypt"),
            "\"/usr/local/bin/git-agecrypt\""
        );
    }

    #[test]
    fn quote_handles_spaces() {
        assert_eq!(
            shell_quote("/Users/Some Body/git-agecrypt"),
            "\"/Users/Some Body/git-agecrypt\"",
        );
    }

    #[test]
    fn quote_escapes_double_quote() {
        assert_eq!(shell_quote(r#"weird"name"#), r#""weird\"name""#);
    }

    #[test]
    fn quote_escapes_backslash_dollar_backtick() {
        // sh would otherwise interpolate `$VAR`, run a `command` substitution,
        // or treat `\` as an escape introducer.
        assert_eq!(shell_quote(r"a\b"), r#""a\\b""#);
        assert_eq!(shell_quote("$VAR"), r#""\$VAR""#);
        assert_eq!(shell_quote("`cmd`"), r#""\`cmd\`""#);
    }
}
