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

#[derive(Debug)]
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

/// Serve requests on stdin/stdout until git closes the pipe.
pub(crate) fn serve<C: Context>(cmd: &CommandContext<C>) -> Result<()> {
    let mut r = BufReader::new(io::stdin().lock());
    let mut w = BufWriter::new(io::stdout().lock());
    serve_io(cmd, &mut r, &mut w)
}

/// The protocol itself, over any pair of streams.
fn serve_io<C: Context>(
    cmd: &CommandContext<C>,
    r: &mut impl Read,
    w: &mut impl Write,
) -> Result<()> {
    handshake(r, w)?;
    while let Some(headers) = read_list(r)? {
        let get = |key: &str| {
            headers
                .iter()
                .find_map(|h| h.strip_prefix(key)?.strip_prefix('='))
                .map(str::to_owned)
        };
        let command = get("command").context("filter request without command")?;
        let path = get("pathname").context("filter request without pathname")?;
        let content = read_content(r)?;
        let result = match command.as_str() {
            "clean" => cmd.clean_bytes(Path::new(&path), content),
            "smudge" => cmd.smudge_bytes(Path::new(&path), content),
            other => Err(anyhow!("unsupported filter command {other}")),
        };
        match result {
            Ok(out) => {
                write_text(w, "status=success")?;
                write_flush(w)?;
                write_content(w, &out)?;
                // An empty trailing list keeps "status=success".
                write_flush(w)?;
            }
            Err(e) => {
                log::error!("{command} {path}: {e:#}");
                write_text(w, "status=error")?;
                write_flush(w)?;
            }
        }
        w.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::internal::testing::{FOO, Fixture};

    /// What git sends: a pkt-line conversation, assembled in memory.
    struct Client {
        buf: Vec<u8>,
    }

    impl Client {
        fn new() -> Self {
            let mut c = Client { buf: vec![] };
            c.list(&["git-filter-client", "version=2"]);
            c.list(&["capability=clean", "capability=smudge", "capability=delay"]);
            c
        }
        fn list(&mut self, lines: &[&str]) -> &mut Self {
            for l in lines {
                write_text(&mut self.buf, l).unwrap();
            }
            write_flush(&mut self.buf).unwrap();
            self
        }
        fn request(&mut self, command: &str, path: &str, content: &[u8]) -> &mut Self {
            let (c, p) = (format!("command={command}"), format!("pathname={path}"));
            self.list(&[&c, &p]);
            write_content(&mut self.buf, content).unwrap();
            self
        }
    }

    /// The server's answers, split into (status lines, content) per request,
    /// after the two handshake lists.
    fn responses(mut out: &[u8]) -> Vec<(Vec<String>, Vec<u8>)> {
        let hello = read_list(&mut out).unwrap().unwrap();
        assert_eq!(hello, vec!["git-filter-server", "version=2"]);
        let caps = read_list(&mut out).unwrap().unwrap();
        assert_eq!(caps, vec!["capability=clean", "capability=smudge"]);
        let mut all = vec![];
        while let Some(status) = read_list(&mut out).unwrap() {
            if status.iter().any(|s| s == "status=success") {
                let content = read_content(&mut out).unwrap();
                let trailer = read_list(&mut out).unwrap().unwrap();
                assert!(trailer.is_empty());
                all.push((status, content));
            } else {
                all.push((status, vec![]));
            }
        }
        all
    }

    #[test]
    fn serves_clean_then_smudge_in_one_session() {
        let fx = Fixture::new();
        let mut client = Client::new();
        client.request("clean", FOO, b"plain");
        let mut out = vec![];
        serve_io(&fx.cmd, &mut &client.buf[..], &mut out).unwrap();
        let r = responses(&out);
        assert_eq!(r.len(), 1);
        assert!(r[0].1.starts_with(b"age-encryption.org/v1"));
        let ciphertext = r[0].1.clone();

        let mut client = Client::new();
        client.request("smudge", FOO, &ciphertext);
        client.request("clean", FOO, b"plain");
        let mut out = vec![];
        serve_io(&fx.cmd, &mut &client.buf[..], &mut out).unwrap();
        let r = responses(&out);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, b"plain");
        // Same plaintext again: served from the cache the smudge just filled.
        assert_eq!(r[1].1, ciphertext);
    }

    #[test]
    fn a_failing_request_answers_status_error_and_the_session_goes_on() {
        let fx = Fixture::new();
        let mut client = Client::new();
        client.request("smudge", FOO, b"not age at all");
        client.request("frobnicate", FOO, b"");
        client.request("clean", FOO, b"after errors");
        let mut out = vec![];
        serve_io(&fx.cmd, &mut &client.buf[..], &mut out).unwrap();
        let r = responses(&out);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].0, vec!["status=error"]);
        assert_eq!(r[1].0, vec!["status=error"]);
        assert!(r[2].1.starts_with(b"age-encryption.org/v1"));
    }

    #[test]
    fn content_larger_than_one_packet_round_trips_through_the_server() {
        let fx = Fixture::new();
        let big = vec![b'z'; MAX_PAYLOAD * 3 + 17];
        let mut client = Client::new();
        client.request("clean", FOO, &big);
        let mut out = vec![];
        serve_io(&fx.cmd, &mut &client.buf[..], &mut out).unwrap();
        let ct = responses(&out).remove(0).1;
        assert!(ct.len() > MAX_PAYLOAD);
        let mut client = Client::new();
        client.request("smudge", FOO, &ct);
        let mut out = vec![];
        serve_io(&fx.cmd, &mut &client.buf[..], &mut out).unwrap();
        assert_eq!(responses(&out).remove(0).1, big);
    }

    #[test]
    fn requests_without_command_or_pathname_are_rejected() {
        let fx = Fixture::new();
        let mut client = Client::new();
        client.list(&["pathname=x"]);
        let mut out = vec![];
        let err = serve_io(&fx.cmd, &mut &client.buf[..], &mut out).unwrap_err();
        assert!(format!("{err:#}").contains("without command"), "{err:#}");
        let mut client = Client::new();
        client.list(&["command=clean"]);
        let err = serve_io(&fx.cmd, &mut &client.buf[..], &mut out).unwrap_err();
        assert!(format!("{err:#}").contains("without pathname"), "{err:#}");
    }

    #[test]
    fn handshake_rejects_other_clients_and_protocol_versions() {
        let fx = Fixture::new();
        let mut out = vec![];
        let mut c = Client { buf: vec![] };
        c.list(&["git-filter-client", "version=3"]);
        let err = serve_io(&fx.cmd, &mut &c.buf[..], &mut out).unwrap_err();
        assert!(
            format!("{err:#}").contains("unsupported filter protocol"),
            "{err:#}"
        );
        let mut c = Client { buf: vec![] };
        c.list(&["something-else", "version=2"]);
        assert!(serve_io(&fx.cmd, &mut &c.buf[..], &mut out).is_err());
        // No handshake at all.
        assert!(serve_io(&fx.cmd, &mut &b""[..], &mut out).is_err());
        // Handshake but no capabilities.
        let mut c = Client { buf: vec![] };
        c.list(&["git-filter-client", "version=2"]);
        let err = serve_io(&fx.cmd, &mut &c.buf[..], &mut out).unwrap_err();
        assert!(format!("{err:#}").contains("no capabilities"), "{err:#}");
    }

    #[test]
    fn only_the_capabilities_git_offers_are_announced() {
        let fx = Fixture::new();
        let mut c = Client { buf: vec![] };
        c.list(&["git-filter-client", "version=2"]);
        c.list(&["capability=smudge"]);
        let mut out = vec![];
        serve_io(&fx.cmd, &mut &c.buf[..], &mut out).unwrap();
        let mut o = &out[..];
        read_list(&mut o).unwrap();
        assert_eq!(
            read_list(&mut o).unwrap().unwrap(),
            vec!["capability=smudge"]
        );
    }

    #[test]
    fn malformed_packets_are_errors() {
        // A pkt-line shorter than its own header.
        let err = read_pkt(&mut &b"0003"[..]).unwrap_err();
        assert!(
            format!("{err:#}").contains("unexpected pkt-line"),
            "{err:#}"
        );
        // Not hex.
        assert!(read_pkt(&mut &b"zzzz"[..]).is_err());
        // Pipe closed inside a request's header list.
        let mut buf = vec![];
        write_text(&mut buf, "command=clean").unwrap();
        let err = read_list(&mut &buf[..]).unwrap_err();
        assert!(format!("{err:#}").contains("mid-request"), "{err:#}");
        // Pipe closed inside content.
        let mut buf = vec![];
        write_pkt(&mut buf, b"partial").unwrap();
        let err = read_content(&mut &buf[..]).unwrap_err();
        assert!(format!("{err:#}").contains("mid-content"), "{err:#}");
    }

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
