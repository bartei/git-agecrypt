// SPDX-License-Identifier: MPL-2.0

//! git's long-running filter protocol (gitattributes(5), "Long Running Filter
//! Process"). One process serves every clean and smudge of a git command, so
//! identities, and the secret store behind an `identityCommand`, load once.

use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};

use super::internal::CommandContext;
use crate::ctx::Context;

/// Largest pkt-line payload: 65520 bytes minus the 4-byte length header.
const MAX_PAYLOAD: usize = 65516;

enum Pkt {
    Flush,
    Data(Vec<u8>),
}

fn read_pkt(r: &mut impl Read) -> Result<Option<Pkt>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n =
        usize::from_str_radix(std::str::from_utf8(&len)?, 16).context("invalid pkt-line length")?;
    if n == 0 {
        return Ok(Some(Pkt::Flush));
    }
    if n <= 4 {
        bail!("unexpected pkt-line of length {n}");
    }
    let mut buf = vec![0u8; n - 4];
    r.read_exact(&mut buf)?;
    Ok(Some(Pkt::Data(buf)))
}

fn write_pkt(w: &mut impl Write, data: &[u8]) -> Result<()> {
    w.write_all(format!("{:04x}", data.len() + 4).as_bytes())?;
    w.write_all(data)?;
    Ok(())
}

fn write_flush(w: &mut impl Write) -> Result<()> {
    Ok(w.write_all(b"0000")?)
}

fn write_text(w: &mut impl Write, line: &str) -> Result<()> {
    write_pkt(w, format!("{line}\n").as_bytes())
}

/// Text packets up to a flush. `None` when git closed the pipe between requests.
fn read_list(r: &mut impl Read) -> Result<Option<Vec<String>>> {
    let mut out = vec![];
    loop {
        match read_pkt(r)? {
            None if out.is_empty() => return Ok(None),
            None => bail!("git closed the filter pipe mid-request"),
            Some(Pkt::Flush) => return Ok(Some(out)),
            Some(Pkt::Data(d)) => out.push(String::from_utf8(d)?.trim_end_matches('\n').to_owned()),
        }
    }
}

fn read_content(r: &mut impl Read) -> Result<Vec<u8>> {
    let mut out = vec![];
    loop {
        match read_pkt(r)?.context("git closed the filter pipe mid-content")? {
            Pkt::Flush => return Ok(out),
            Pkt::Data(d) => out.extend_from_slice(&d),
        }
    }
}

fn write_content(w: &mut impl Write, data: &[u8]) -> Result<()> {
    for chunk in data.chunks(MAX_PAYLOAD) {
        write_pkt(w, chunk)?;
    }
    write_flush(w)
}

fn handshake(r: &mut impl Read, w: &mut impl Write) -> Result<()> {
    let hello = read_list(r)?.context("git sent no handshake")?;
    if hello.first().map(String::as_str) != Some("git-filter-client")
        || !hello.iter().any(|l| l == "version=2")
    {
        bail!("unsupported filter protocol: {hello:?}");
    }
    write_text(w, "git-filter-server")?;
    write_text(w, "version=2")?;
    write_flush(w)?;
    // git waits for this answer before it sends its capabilities.
    w.flush()?;
    let caps = read_list(r)?.context("git sent no capabilities")?;
    for cap in ["capability=clean", "capability=smudge"] {
        if caps.iter().any(|c| c == cap) {
            write_text(w, cap)?;
        }
    }
    write_flush(w)?;
    Ok(w.flush()?)
}

/// Serve requests until git closes the pipe.
pub(crate) fn serve<C: Context>(cmd: &CommandContext<C>) -> Result<()> {
    let mut r = BufReader::new(io::stdin().lock());
    let mut w = BufWriter::new(io::stdout().lock());
    handshake(&mut r, &mut w)?;
    while let Some(headers) = read_list(&mut r)? {
        let get = |key: &str| {
            headers
                .iter()
                .find_map(|h| h.strip_prefix(key)?.strip_prefix('='))
                .map(str::to_owned)
        };
        let command = get("command").context("filter request without command")?;
        let path = get("pathname").context("filter request without pathname")?;
        let content = read_content(&mut r)?;
        let result = match command.as_str() {
            "clean" => cmd.clean_bytes(Path::new(&path), content),
            "smudge" => cmd.smudge_bytes(Path::new(&path), content),
            other => Err(anyhow!("unsupported filter command {other}")),
        };
        match result {
            Ok(out) => {
                write_text(&mut w, "status=success")?;
                write_flush(&mut w)?;
                write_content(&mut w, &out)?;
                // An empty trailing list keeps "status=success".
                write_flush(&mut w)?;
            }
            Err(e) => {
                log::error!("{command} {path}: {e:#}");
                write_text(&mut w, "status=error")?;
                write_flush(&mut w)?;
            }
        }
        w.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkt_round_trip_and_flush() {
        let mut buf = vec![];
        write_text(&mut buf, "command=smudge").unwrap();
        write_flush(&mut buf).unwrap();
        assert_eq!(&buf[..4], b"0013");
        let list = read_list(&mut &buf[..]).unwrap().unwrap();
        assert_eq!(list, vec!["command=smudge".to_owned()]);
    }

    #[test]
    fn content_splits_at_the_payload_limit() {
        let data = vec![7u8; MAX_PAYLOAD * 2 + 3];
        let mut buf = vec![];
        write_content(&mut buf, &data).unwrap();
        assert_eq!(read_content(&mut &buf[..]).unwrap(), data);
    }

    #[test]
    fn eof_between_requests_ends_the_session() {
        assert!(read_list(&mut &b""[..]).unwrap().is_none());
    }
}
