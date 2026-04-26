use std::{
    io::{self, ErrorKind as IoErrorKind, Read},
    path::Path,
};

use age::{
    DecryptError, Decryptor, Encryptor, Identity, Recipient,
    armor::ArmoredReader,
    cli_common::{StdinGuard, UiCallbacks, read_identities},
    plugin::{self, RecipientPluginV1},
};
use anyhow::{Context, Result, bail};

pub(crate) fn decrypt(
    identities: &[impl AsRef<Path>],
    encrypted: &mut impl Read,
) -> Result<Option<Vec<u8>>> {
    let id = load_identities(identities)?;
    let id = id.iter().map(|i| i.as_ref() as &dyn Identity);
    let mut decrypted = vec![];
    let decryptor = match Decryptor::new(ArmoredReader::new(encrypted)) {
        Ok(d) if d.is_scrypt() => bail!("Passphrase encrypted files are not supported"),
        Ok(d) => d,
        Err(DecryptError::InvalidHeader) => return Ok(None),
        Err(DecryptError::Io(e)) => {
            match e.kind() {
                // Age gives unexpected EOF when the file contains not enough data
                IoErrorKind::UnexpectedEof => return Ok(None),
                _ => bail!(e),
            }
        }
        Err(e) => {
            log::error!("Decryption error: {e:?}");
            bail!(e)
        }
    };

    let mut reader = decryptor.decrypt(id)?;
    reader.read_to_end(&mut decrypted)?;
    Ok(Some(decrypted))
}

fn load_identities(identities: &[impl AsRef<Path>]) -> Result<Vec<Box<dyn Identity>>> {
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
            plugin_recipients.push(recipient);
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
}
