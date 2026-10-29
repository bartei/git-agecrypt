use std::{
    io::{self, ErrorKind as IoErrorKind, Read},
    path::Path,
};

use age::{
    DecryptError, Decryptor, Encryptor, Identity, IdentityFile, NoCallbacks, Recipient,
    armor::ArmoredReader,
    cli_common::{StdinGuard, UiCallbacks, read_identities},
    plugin::{self, RecipientPluginV1},
};
use anyhow::{Context, Result, bail};

#[cfg(test)]
pub(crate) fn decrypt(
    identities: &[impl AsRef<Path>],
    encrypted: &mut impl Read,
) -> Result<Option<Vec<u8>>> {
    let mut data = vec![];
    encrypted.read_to_end(&mut data)?;
    decrypt_with(&load_identities(identities)?, &data)
}

type SliceDecryptor<'a> = Decryptor<ArmoredReader<io::BufReader<&'a [u8]>>>;

fn decryptor(data: &[u8]) -> Result<Option<SliceDecryptor<'_>>> {
    match Decryptor::new(ArmoredReader::new(data)) {
        Ok(d) if d.is_scrypt() => bail!("Passphrase encrypted files are not supported"),
        Ok(d) => Ok(Some(d)),
        Err(DecryptError::InvalidHeader) => Ok(None),
        // Age gives unexpected EOF when the file contains not enough data
        Err(DecryptError::Io(e)) if e.kind() == IoErrorKind::UnexpectedEof => Ok(None),
        Err(e) => {
            log::error!("Decryption error: {e:?}");
            bail!(e)
        }
    }
}

/// Decrypt with identities already in memory, so a long-running filter loads
/// them once. `None` when `data` is not age. Each identity is tried on its own:
/// in one decryptor, a plugin identity that errors (absent plugin, another
/// holder's key) aborts before the working key behind it is even tried.
pub(crate) fn decrypt_with(
    identities: &[Box<dyn Identity>],
    data: &[u8],
) -> Result<Option<Vec<u8>>> {
    if decryptor(data)?.is_none() {
        return Ok(None);
    }
    let mut failure = None;
    for identity in identities {
        let decryptor = decryptor(data)?.context("age header vanished")?;
        match decryptor.decrypt(std::iter::once(identity.as_ref() as &dyn Identity)) {
            Ok(mut reader) => {
                let mut decrypted = vec![];
                reader.read_to_end(&mut decrypted)?;
                return Ok(Some(decrypted));
            }
            Err(DecryptError::NoMatchingKeys) => {}
            Err(e) => failure = Some(e),
        }
    }
    Err(match failure {
        Some(e) => anyhow::anyhow!(e).context("no identity could decrypt; one of them failed"),
        None => anyhow::anyhow!(DecryptError::NoMatchingKeys),
    })
}

/// Identities from the text an `identityCommand` printed (an identity file).
pub(crate) fn identities_from_text(text: &str) -> Result<Vec<Box<dyn Identity>>> {
    let ids = IdentityFile::from_buffer(io::Cursor::new(text.as_bytes()))
        .context("identityCommand output is not an age identity file")?
        .with_callbacks(NoCallbacks)
        .into_identities()
        .context("identityCommand output holds no usable identity")?;
    if ids.is_empty() {
        bail!("identityCommand printed no identity");
    }
    Ok(ids)
}

/// True when `data` is an age file whose line endings were rewritten to CRLF.
/// git does that to blobs it takes for text (a short ciphertext with no NUL or
/// control byte) when `core.autocrlf` is on and the path is not `-text`; the
/// result is not valid age any more.
pub(crate) fn crlf_rewritten(data: &[u8]) -> bool {
    data.starts_with(b"age-encryption.org/v1\r\n")
        || data.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----\r\n")
}

pub(crate) fn load_identities(identities: &[impl AsRef<Path>]) -> Result<Vec<Box<dyn Identity>>> {
    // age::cli_common::read_identities takes Vec<String>, so the path has
    // to round-trip through UTF-8. Lossy conversion would silently change
    // the bytes age then opens — fail explicitly instead.
    let id: Vec<String> = identities
        .iter()
        .map(|i| {
            let p = i.as_ref();
            p.to_str()
                .map(str::to_owned)
                .with_context(|| format!("Identity path {} is not valid UTF-8", p.display()))
        })
        .collect::<Result<_>>()?;
    let mut stdin_guard = StdinGuard::new(false);
    let rv = read_identities(id.clone(), None, &mut stdin_guard)
        .with_context(|| format!("Loading identities failed from paths: {id:?}"))?;
    Ok(rv)
}

pub(crate) fn encrypt(
    public_keys: &[impl AsRef<str> + std::fmt::Debug],
    cleartext: &mut impl Read,
) -> Result<Vec<u8>> {
    let recipients = load_public_keys(public_keys)?;

    let recipient_refs = recipients.iter().map(|r| r.as_ref() as &dyn Recipient);
    let encryptor = Encryptor::with_recipients(recipient_refs).with_context(|| {
        format!("Couldn't load keys for recipients; public_keys={public_keys:?}")
    })?;
    let mut encrypted = vec![];

    let mut writer = encryptor.wrap_output(&mut encrypted)?;
    io::copy(cleartext, &mut writer)?;
    writer.finish()?;
    Ok(encrypted)
}

fn load_public_keys(public_keys: &[impl AsRef<str>]) -> Result<Vec<Box<dyn Recipient + Send>>> {
    let mut recipients: Vec<Box<dyn Recipient + Send>> = vec![];
    let mut plugin_recipients = vec![];

    for pubk in public_keys {
        if let Ok(pk) = pubk.as_ref().parse::<age::x25519::Recipient>() {
            recipients.push(Box::new(pk));
        } else if let Ok(pk) = pubk.as_ref().parse::<age::ssh::Recipient>() {
            recipients.push(Box::new(pk));
        } else if let Ok(recipient) = pubk.as_ref().parse::<plugin::Recipient>() {
            // `age1tag1…` / `age1tagpq1…` are *native* tagged recipients
            // (the standardized format emitted by recent age-plugin-tpm and
            // age-plugin-se, for hardware-backed keys), not plugin recipients.
            // age's plugin parser still accepts them because it reads the
            // bech32 HRP (`age1tag` / `age1tagpq`) as a plugin name — so
            // without this guard we'd go on to spawn a nonexistent
            // `age-plugin-tag` binary and fail with a confusing error.
            //
            // Native support lives in `age::tag::Recipient` /
            // `age::tagpq::Recipient`, which are unreleased upstream (the
            // latest published `age` is 0.11.x, which lacks them). Once a
            // release ships those types, parse the recipient natively here
            // instead of bailing. Tracking:
            // https://github.com/bartei/git-agecrypt/issues/17
            match recipient.plugin() {
                "tag" | "tagpq" => bail!(
                    "Tagged recipients (`{}`) — the format produced by recent \
                     age-plugin-tpm / age-plugin-se for hardware-backed keys — are not \
                     yet supported by git-agecrypt. Native tagged-recipient support is \
                     pending an upstream `age` crate release (the latest published \
                     version lacks `age::tag::Recipient`). Track progress at \
                     https://github.com/bartei/git-agecrypt/issues/17",
                    pubk.as_ref()
                ),
                _ => plugin_recipients.push(recipient),
            }
        } else {
            bail!("Invalid recipient");
        }
    }
    let callbacks = UiCallbacks {};

    for plugin_name in plugin_recipients.iter().map(|r| r.plugin()) {
        let recipient = RecipientPluginV1::new(plugin_name, &plugin_recipients, &[], callbacks)?;
        recipients.push(Box::new(recipient));
    }

    Ok(recipients)
}

pub(crate) fn validate_public_keys(public_keys: &[impl AsRef<str>]) -> Result<()> {
    load_public_keys(public_keys)?;
    Ok(())
}

pub(crate) fn validate_identity(identity: impl AsRef<Path>) -> Result<()> {
    let p = identity.as_ref();
    let id_str = p
        .to_str()
        .with_context(|| format!("Identity path {} is not valid UTF-8", p.display()))?
        .to_owned();
    let mut stdin_guard = StdinGuard::new(false);
    read_identities(vec![id_str], None, &mut stdin_guard)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::age::secrecy::ExposeSecret;
    use assert_fs::TempDir;
    use std::io::Cursor;

    fn keypair() -> (::age::x25519::Identity, String, String) {
        let id = ::age::x25519::Identity::generate();
        let public = id.to_public().to_string();
        let secret = id.to_string().expose_secret().to_string();
        (id, public, secret)
    }

    fn write_identity(dir: &TempDir, secret: &str) -> std::path::PathBuf {
        let path = dir.path().join("id.key");
        std::fs::write(&path, secret).unwrap();
        path
    }

    #[test]
    fn round_trip_x25519() {
        let dir = TempDir::new().unwrap();
        let (_id, public, secret) = keypair();
        let id_path = write_identity(&dir, &secret);

        let plaintext = b"the quick brown fox jumps over the lazy dog";
        let ciphertext = encrypt(&[public], &mut &plaintext[..]).unwrap();
        assert_ne!(&ciphertext[..], &plaintext[..]);

        let mut cur = Cursor::new(ciphertext);
        let decrypted = decrypt(&[id_path], &mut cur).unwrap().unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn decrypt_returns_none_on_invalid_header() {
        let dir = TempDir::new().unwrap();
        let (_id, _public, secret) = keypair();
        let id_path = write_identity(&dir, &secret);

        // Random non-age content — decrypt must report "not encrypted"
        // (Ok(None)) rather than erroring out, so callers can fall back
        // to passing through plaintext (e.g. textconv on working-copy files).
        let mut cur = Cursor::new(b"this is not age encrypted content".to_vec());
        let result = decrypt(&[id_path], &mut cur).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn decrypt_returns_none_on_short_input() {
        // Less data than even an age header would occupy — must round-trip
        // as Ok(None) via the UnexpectedEof branch.
        let dir = TempDir::new().unwrap();
        let (_id, _public, secret) = keypair();
        let id_path = write_identity(&dir, &secret);

        let mut cur = Cursor::new(b"".to_vec());
        let result = decrypt(&[id_path], &mut cur).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn decrypt_with_wrong_identity_errors() {
        let dir = TempDir::new().unwrap();
        let (_id_a, public_a, _) = keypair();
        let (_id_b, _public_b, secret_b) = keypair();
        let other_id_path = write_identity(&dir, &secret_b);

        // Encrypt to A, try to decrypt with B — must fail loudly rather
        // than returning empty plaintext.
        let ciphertext = encrypt(&[public_a], &mut &b"secret"[..]).unwrap();
        let mut cur = Cursor::new(ciphertext);
        let result = decrypt(&[other_id_path], &mut cur);
        assert!(result.is_err(), "wrong identity must error on decrypt");
    }

    #[test]
    fn validate_public_keys_accepts_x25519() {
        let (_id, public, _) = keypair();
        validate_public_keys(&[public]).unwrap();
    }

    #[test]
    fn validate_public_keys_rejects_garbage() {
        let result = validate_public_keys(&["this-is-not-a-recipient"]);
        assert!(result.is_err());
    }

    #[test]
    fn validate_public_keys_rejects_tagged_recipient_with_clear_message() {
        // `age1tag1…` is a native tagged recipient (hardware-backed keys via
        // age-plugin-tpm / age-plugin-se), which the published `age` crate
        // can't yet encrypt to. age's plugin parser accepts it as plugin
        // name "tag", so without our guard the tool would try to spawn a
        // nonexistent `age-plugin-tag` binary. Assert we fail with an
        // actionable message instead — and crucially do NOT mention a
        // missing plugin binary. Recipient from age-plugin-tpm's docs.
        let tag = "age1tag1q096edfp3ty6n36fj5kyq0yuesp7rdcmm7sjswzdcrekh6ash8n3uys987t";
        let err = validate_public_keys(&[tag]).expect_err("tagged recipient must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("Tagged recipients"),
            "error should explain tagged recipients aren't supported: {msg}"
        );
        assert!(
            msg.contains("bartei/git-agecrypt/issues/17"),
            "error should point at the tracking issue: {msg}"
        );
        assert!(
            !msg.contains("age-plugin-tag"),
            "error must not blame a missing plugin binary: {msg}"
        );
    }

    #[test]
    fn validate_identity_accepts_real_key() {
        let dir = TempDir::new().unwrap();
        let (_id, _public, secret) = keypair();
        let id_path = write_identity(&dir, &secret);
        validate_identity(&id_path).unwrap();
    }

    #[test]
    fn validate_identity_rejects_garbage() {
        let dir = TempDir::new().unwrap();
        let path = write_identity(&dir, "not an identity\n");
        let result = validate_identity(&path);
        assert!(result.is_err());
    }

    #[test]
    fn validate_identity_rejects_missing_file() {
        let result = validate_identity("/this/path/does/not/exist");
        assert!(result.is_err());
    }

    #[test]
    fn encrypt_with_no_recipients_errors() {
        // Passing the empty slice must surface a clear error instead of
        // silently producing a "ciphertext" anyone can read.
        let recipients: [&str; 0] = [];
        let result = encrypt(&recipients, &mut &b"secret"[..]);
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn validate_identity_rejects_non_utf8_path() {
        // On Unix, OsStr can hold arbitrary bytes; we surface a clear
        // error rather than silently lossy-converting (which would feed
        // age the wrong filename).
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let bytes: &[u8] = b"/tmp/\xff\xfe-not-utf8";
        let os = OsStr::from_bytes(bytes);
        let path = std::path::Path::new(os);
        let err = validate_identity(path).expect_err("non-UTF8 path must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("not valid UTF-8"),
            "error must mention UTF-8: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn decrypt_with_non_utf8_identity_path_errors() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let bytes: &[u8] = b"/tmp/\xff\xfe-not-utf8";
        let os = OsStr::from_bytes(bytes);
        let path = std::path::PathBuf::from(os);
        let mut cur = Cursor::new(b"".to_vec());
        let err = decrypt(&[path], &mut cur).expect_err("non-UTF8 identity path must error");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("not valid UTF-8"),
            "error must mention UTF-8: {msg}"
        );
    }

    #[test]
    fn identities_from_text_reads_an_identity_file() {
        let (_id, public, secret) = keypair();
        let ids = identities_from_text(&format!("# created: now\n{secret}\n")).unwrap();
        let ciphertext = encrypt(&[public], &mut &b"secret"[..]).unwrap();
        assert_eq!(decrypt_with(&ids, &ciphertext).unwrap().unwrap(), b"secret");
    }

    #[test]
    fn identities_from_text_refuses_output_without_identity() {
        assert!(identities_from_text("").is_err());
        assert!(identities_from_text("# only a comment\n").is_err());
    }

    #[test]
    fn crlf_rewritten_detects_both_age_formats() {
        assert!(crlf_rewritten(
            b"age-encryption.org/v1\r\n-> X25519 abc\r\n"
        ));
        assert!(crlf_rewritten(
            b"-----BEGIN AGE ENCRYPTED FILE-----\r\nYWdl\r\n"
        ));
        assert!(!crlf_rewritten(b"age-encryption.org/v1\n-> X25519 abc\n"));
        assert!(!crlf_rewritten(
            b"-----BEGIN AGE ENCRYPTED FILE-----\nYWdl\n"
        ));
        assert!(!crlf_rewritten(b"plain text\r\n"));
        assert!(!crlf_rewritten(b""));
    }

    /// An identity that errors when asked to unwrap, the way a plugin identity
    /// does when its plugin is missing: a passphrase-protected identity file
    /// with no callback that could supply the passphrase.
    fn failing_identity() -> Box<dyn Identity> {
        use ::age::secrecy::SecretString;
        use std::io::Write;
        let (_id, _public, secret) = keypair();
        let mut encrypted = vec![];
        let mut w = Encryptor::with_user_passphrase(SecretString::from("pw".to_owned()))
            .wrap_output(&mut encrypted)
            .unwrap();
        w.write_all(secret.as_bytes()).unwrap();
        w.finish().unwrap();
        let id = ::age::encrypted::Identity::from_buffer(
            io::Cursor::new(encrypted),
            None,
            NoCallbacks,
            None,
        )
        .unwrap()
        .expect("a passphrase-encrypted identity file");
        Box::new(id)
    }

    #[test]
    fn decrypt_with_tries_the_next_identity_after_a_failing_one() {
        let (good, public, _) = keypair();
        let ct = encrypt(&[public], &mut &b"secret"[..]).unwrap();
        let ids: Vec<Box<dyn Identity>> = vec![failing_identity(), Box::new(good)];
        assert_eq!(decrypt_with(&ids, &ct).unwrap().unwrap(), b"secret");
    }

    #[test]
    fn decrypt_with_reports_the_failure_when_nothing_opens() {
        let (_good, public, _) = keypair();
        let ct = encrypt(&[public], &mut &b"secret"[..]).unwrap();
        let err = decrypt_with(&[failing_identity()], &ct).unwrap_err();
        assert!(format!("{err:#}").contains("one of them failed"), "{err:#}");
    }

    #[test]
    fn decrypt_with_only_wrong_identities_reports_no_matching_keys() {
        let (_a, public_a, _) = keypair();
        let (b, _public_b, _) = keypair();
        let ct = encrypt(&[public_a], &mut &b"secret"[..]).unwrap();
        let ids: Vec<Box<dyn Identity>> = vec![Box::new(b)];
        let err = decrypt_with(&ids, &ct).unwrap_err();
        let msg = format!("{err:#}");
        assert!(!msg.contains("one of them failed"), "{msg}");
        // And with no identity at all the same, instead of a panic or `None`.
        assert!(decrypt_with(&[], &ct).is_err());
    }

    #[test]
    fn decrypt_with_errors_on_malformed_age_headers_instead_of_passing_them_through() {
        // Looks like age but is not parseable: that must be an error, never
        // `None` (which callers take for plaintext and would pass through).
        for header in [
            &b"age-encryption.org/v1\nthis is not a stanza\n"[..],
            &b"age-encryption.org/v9\n-> X25519 abc\n--- mac\n"[..],
        ] {
            let mut data = header.to_vec();
            data.extend(std::iter::repeat_n(b'x', 512));
            let err = decrypt_with(&[], &data).unwrap_err();
            assert!(format!("{err:#}").contains("Unknown age format"), "{err:#}");
        }
    }

    #[test]
    fn decrypt_with_rejects_passphrase_encrypted_input() {
        use ::age::secrecy::SecretString;
        use std::io::Write;
        let mut ct = vec![];
        let mut w = Encryptor::with_user_passphrase(SecretString::from("pw".to_owned()))
            .wrap_output(&mut ct)
            .unwrap();
        w.write_all(b"secret").unwrap();
        w.finish().unwrap();
        let err = decrypt_with(&[], &ct).unwrap_err();
        assert!(format!("{err:#}").contains("Passphrase"), "{err:#}");
    }

    #[test]
    fn encrypt_accepts_an_ssh_ed25519_recipient() {
        // Throwaway key generated for this test; only the public half exists.
        let ssh =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJOP0uhCI/Ct0WKbnMnsY6zveQnXo3Vqw6zM6PXW24OL test";
        let ct = encrypt(&[ssh], &mut &b"secret"[..]).unwrap();
        assert!(ct.starts_with(b"age-encryption.org/v1"));
        validate_public_keys(&[ssh]).unwrap();
    }
}
