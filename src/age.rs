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
    check_recipient_mix(public_keys)?;
    let mut recipients: Vec<Box<dyn Recipient + Send>> = vec![];
    let mut plugin_recipients = vec![];

    for pubk in public_keys {
        if let Ok(pk) = pubk.as_ref().parse::<age::x25519::Recipient>() {
            recipients.push(Box::new(pk));
        } else if let Ok(pk) = pubk.as_ref().parse::<age::ssh::Recipient>() {
            recipients.push(Box::new(pk));
        } else if let Ok(pk) = pubk.as_ref().parse::<age::tag::Recipient>() {
            // Native tagged recipients (`age1tag1…`), the standardized format
            // emitted by age-plugin-tpm / age-plugin-se for hardware-backed
            // keys. Must be tried before the plugin parser, which would
            // otherwise read the `age1tag` HRP as a plugin named "tag".
            recipients.push(Box::new(pk));
        } else if let Ok(pk) = pubk.as_ref().parse::<age::tagpq::Recipient>() {
            // Post-quantum variant (`age1tagpq1…`), same plugin-HRP caveat.
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

/// Rejects a recipient set that mixes post-quantum (`age1tagpq1…`) and classic
/// (x25519, SSH, `age1tag1…`) recipients. age refuses to encrypt to such a set
/// — a classic recipient would void the post-quantum protection — but only
/// fails at encryption time (i.e. during `git add`); checking here lets
/// `config add` fail up front. Plugin recipients only declare their labels
/// when encrypting, so they're skipped and left to age's own check.
pub(crate) fn check_recipient_mix(public_keys: &[impl AsRef<str>]) -> Result<()> {
    let (mut pq, mut classic) = (false, false);
    for pubk in public_keys {
        let pubk = pubk.as_ref();
        if pubk.parse::<age::tagpq::Recipient>().is_ok() {
            pq = true;
        } else if pubk.parse::<age::x25519::Recipient>().is_ok()
            || pubk.parse::<age::ssh::Recipient>().is_ok()
            || pubk.parse::<age::tag::Recipient>().is_ok()
        {
            classic = true;
        }
    }
    if pq && classic {
        bail!(
            "Post-quantum recipients (`age1tagpq1…`) can't be combined with classic \
             recipients (x25519, SSH or `age1tag1…`) for the same file: age refuses to \
             encrypt to both, as the classic recipient would void the post-quantum \
             protection. Use only post-quantum recipients for this file, or drop the \
             `age1tagpq1…` one."
        );
    }
    Ok(())
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

/// `age1tagpq1…` recipient from the `age` crate's own test vectors.
#[cfg(test)]
pub(crate) const TEST_TAGPQ_RECIPIENT: &str = "age1tagpq1m3e4wvp6hzcrn9exhy0ae3xfx2sjymp594k3tg7j4dpmj922we65vtnmrt2pyallax8669zqkr2pmfchptr4n38kug2xmcmp3adk2lnjqu00x5kxz5pvhmrltvfh9wuq973pcx35cnq8syn9qd3tzpehgztl4xpzr3tpd67g8af9trnjpc05gh7wu536aq4qt2y8zhsm4tvrfpsfl36qs5fpzysnk3sp9w77qzeg49357xex40v4s2lvt620swyys7u8yxdcnu4rkkwxdmt55gsuc3h5c5swahnegjgqwc60hn085ec3sjztwm45l44y3j2at9t6v9zra4ek3kek6waecqm98yaxl37w0d2zra626nz63jdm5sg59w7lyptw83zm6fntd8d0x03a9z6h9prfgpygzar6zrxjcrt4cdctk2mhf95s4a6v4zklfd49xhpsaeujm57thx2x3e3hwzc86ftfhmq5mkxxz3d6r8ws24xj4qfn73eyezg2wy094e3why592pghz27ruq3vkyegrv80eftnw9wqzwgvnwyseaus0yt84fylzrpzp6x2fguxuqjmgudr8xd33qm30evdpxd3jvjg8qh4q60kyq80jgff369k7nrepdc38grd2dava520excqp0ey0x39khx8ry03yffcatgv84fsx5j49djpapedsy693zute5xv5g2ewzrlj5se7akvkc4g4vmzhputpq8eyj9wz5dz6qtn7g3cfpd95nahw4ytspan0feyye04dcylv24ege7zkaj004gjwcxqxfqu2quawa83sx452jqjn8t48czp0xspwgnmvjyhttzzy6nhq8xzkdwnvsfefkwva6asrqc93zjn4rly5gnlv93xy3uzmr39szvjnf63426qzyeyvguc4vdcquwgsxgq236afcpqz866ny4tn7ckc0umefj242rt5vtvwqzzrvfev2mpvqcufp9pqvefyv4ftyuhgausfzuaadsczeykmft5wv3frzgrcp9ztr93h478ke4t86spp2uhyjkj73mp9g92ddk2fpv7v3njzsqgwhq3789sqrgkskehn0zjscckhwftyq4vet7vrlx2hs5kd9cwnq6t0djffhh3zquh4j3p0yaj9z2rc9wykg0usqw7983rrgur9jg8rnnqypwcz2lyclnnc705fc5g3an93ps60q6mxqp85u0ewtxdjlqcks84yduft0a0g6e7naew3v9u2d08knarvajn8q3gq9pgxde3s7nx94lus48wwvw2xjm7k82tvylec2393jdsuvch2xpe77w8hpv9nvsxfsrs270njpmfvpmgyk2cffl9tjp3qqcc4dfkf5rme2dg0x7ew8g39www5smm705q5da4eqvnqwrkavtq6xje9ss38hnkglz4eddz8f5qruvqmq2ff9l22gwkv8h432rdkysy0grkul8e2fedvkyyapfxt760udcgu92m54wl9yavmj4ga3ph9r5n99cjrq6wj5v33x33fe5vkjvfwnnt40wuv2hyexc9f4ylyqv9ldqq9epd4yuv8vrsfx2qy2kqz08kqhnzspy6s0x8fa5c2xkg5y2q0rvz4vnk7rp0acg6eksc3t7cxnn8y7glkjsqja3p56uz6vvhcw55d3ysad0hvsqxpjnc7svenf2gc5xn5kyr0et2vvyruxlnpqcdpqh9pzplumy5yzjxftyzh9ujfw0jq7ee60zx2x23p0jzyh9dvmly8p9h9ysptlqu7kwnejd65dnr75a0np2fvke8xen38r57w6z3wz3mycjmmn267wwxndfh9jdps7uxtct2wwfgamkpa5ap8s96lhfjztpwcm6fguhphu38yunu2v4vz3syzrvgwtqpemkewzp766nyu6texxvjlaemnhyyqutkcy6a42vqfsz49rw5wr4gt70r4vdaasehqjg46fnyts4sthrxadfllha3avu49wsj2c4jx";

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
    fn encrypt_to_tagged_recipient() {
        // `age1tag1…` is a native tagged recipient (hardware-backed keys via
        // age-plugin-tpm / age-plugin-se). age's plugin parser would read it
        // as plugin "tag" and try to spawn a nonexistent `age-plugin-tag`,
        // so assert it is encrypted to natively. Recipient from
        // age-plugin-tpm's docs.
        let tag = "age1tag1q096edfp3ty6n36fj5kyq0yuesp7rdcmm7sjswzdcrekh6ash8n3uys987t";
        validate_public_keys(&[tag]).unwrap();
        let ciphertext = encrypt(&[tag], &mut &b"secret"[..]).unwrap();
        let header = String::from_utf8_lossy(&ciphertext);
        assert!(
            header.contains("-> p256tag "),
            "expected a p256tag stanza in the header: {header}"
        );
    }

    #[test]
    fn encrypt_to_tagpq_recipient() {
        validate_public_keys(&[TEST_TAGPQ_RECIPIENT]).unwrap();
        let ciphertext = encrypt(&[TEST_TAGPQ_RECIPIENT], &mut &b"secret"[..]).unwrap();
        let header = String::from_utf8_lossy(&ciphertext);
        assert!(
            header.contains("-> mlkem768p256tag "),
            "expected a mlkem768p256tag stanza in the header: {header}"
        );
    }

    #[test]
    fn rejects_post_quantum_mixed_with_classic() {
        let (_id, public, _) = keypair();
        let err = validate_public_keys(&[TEST_TAGPQ_RECIPIENT, public.as_str()])
            .expect_err("PQ + classic must be rejected");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("can't be combined with classic"),
            "error should explain the PQ/classic mix: {msg}"
        );
    }

    #[test]
    fn accepts_classic_mix_of_x25519_and_tag() {
        let (_id, public, _) = keypair();
        let tag = "age1tag1q096edfp3ty6n36fj5kyq0yuesp7rdcmm7sjswzdcrekh6ash8n3uys987t";
        encrypt(&[public.as_str(), tag], &mut &b"secret"[..]).unwrap();
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
