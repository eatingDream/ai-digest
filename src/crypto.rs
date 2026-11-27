use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{bufread::GzDecoder, Compression, GzBuilder};
use html5gum::Token;
use rand::{rngs::OsRng, RngCore};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    assets,
    i18n::{self, Lang},
    model::{
        atomic_write_text, clean_text, escape_json_for_script, files_under, html_escape as esc,
        html_tokens, is_protected_security_header, normalized_safe_url,
        parse_header_removal_target, strip_marked, validate_slug, DigestPaths, FileLock, LangAlt,
    },
};

pub const PBKDF2_ITERATIONS: u32 = 600_000;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const KEY_LEN: usize = 32;
pub const MAX_INLINE_JS_BYTES: usize = 30 * 1024;
pub const MAX_DECOMPRESSED_BYTES: usize = 32 * 1024 * 1024;
const ENVELOPE_SLOT: &str = "__ENCRYPT_CFG_JSON__";
const DATA_SCRIPT_TYPES: &[&[u8]] = &[
    b"application/json",
    b"application/ld+json",
    b"speculationrules",
];
const EXECUTABLE_ASSETS: &[&str] = &[
    assets::BOOT_JS,
    assets::READER_JS,
    assets::INDEX_JS,
    assets::UNLOCK_JS,
    assets::LOCK_JS,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub v: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z: Option<String>,
    pub iter: u32,
    pub salt: String,
    pub iv: String,
    pub ct: String,
}

pub fn derive_key(password: &str, salt: &[u8]) -> [u8; KEY_LEN] {
    derive(password, salt, PBKDF2_ITERATIONS)
}

fn derive(password: &str, salt: &[u8], iterations: u32) -> [u8; KEY_LEN] {
    let mut key = [0; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut key);
    key
}

pub fn encrypt_payload(plain: &[u8], key: &[u8; KEY_LEN], salt: &[u8]) -> Result<Envelope> {
    let mut nonce = [0; NONCE_LEN];
    OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| anyhow!("cannot generate encryption nonce"))?;
    encrypt_payload_with_nonce(plain, key, salt, nonce)
}

pub fn encrypt_payload_with_nonce(
    plain: &[u8],
    key: &[u8; KEY_LEN],
    salt: &[u8],
    nonce: [u8; NONCE_LEN],
) -> Result<Envelope> {
    ensure!(salt.len() == SALT_LEN, "invalid encryption salt length");
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    encoder.write_all(plain)?;
    let compressed = encoder.finish()?;
    let cipher = Aes256Gcm::new(key.into());
    let ct = cipher
        .encrypt(&Nonce::from(nonce), compressed.as_slice())
        .map_err(|_| anyhow!("AES-GCM encryption failed"))?;
    Ok(Envelope {
        v: 2,
        z: Some("gzip".into()),
        iter: PBKDF2_ITERATIONS,
        salt: STANDARD.encode(salt),
        iv: STANDARD.encode(nonce),
        ct: STANDARD.encode(ct),
    })
}

fn check_envelope(envelope: &Envelope) -> Result<(Vec<u8>, [u8; NONCE_LEN], Vec<u8>)> {
    match (envelope.v, envelope.z.as_deref()) {
        (1, None) | (2, Some("gzip")) => {}
        (1, Some(_)) => bail!("unexpected compression field in v1 encryption envelope"),
        (2, _) => bail!("unexpected encryption compression"),
        _ => bail!("unsupported encryption envelope version"),
    }
    ensure!(
        envelope.iter == PBKDF2_ITERATIONS,
        "unexpected PBKDF2 iteration count"
    );
    let salt = STANDARD
        .decode(&envelope.salt)
        .context("invalid encryption salt")?;
    ensure!(salt.len() == SALT_LEN, "invalid encryption salt length");
    let iv = STANDARD
        .decode(&envelope.iv)
        .context("invalid encryption nonce")?
        .try_into()
        .map_err(|_| anyhow!("invalid encryption nonce length"))?;
    let ct = STANDARD
        .decode(&envelope.ct)
        .context("invalid ciphertext")?;
    ensure!(ct.len() >= 16, "ciphertext is too short");
    Ok((salt, iv, ct))
}

pub fn decrypt_envelope(envelope: &Envelope, passphrase: &str) -> Result<Vec<u8>> {
    let (salt, iv, ct) = check_envelope(envelope)?;
    decrypt_payload(
        envelope.v,
        &derive(passphrase, &salt, envelope.iter),
        iv,
        &ct,
    )
}

pub fn decrypt_envelope_with_key(envelope: &Envelope, key: &[u8; KEY_LEN]) -> Result<Vec<u8>> {
    let (_, iv, ct) = check_envelope(envelope)?;
    decrypt_payload(envelope.v, key, iv, &ct)
}

fn decrypt_payload(
    version: u8,
    key: &[u8; KEY_LEN],
    iv: [u8; NONCE_LEN],
    ct: &[u8],
) -> Result<Vec<u8>> {
    let packed = Aes256Gcm::new(key.into())
        .decrypt(&Nonce::from(iv), ct)
        .map_err(|_| anyhow!("AES-GCM authentication failed"))?;
    if version == 1 {
        ensure!(
            packed.len() <= MAX_DECOMPRESSED_BYTES,
            "decrypted payload exceeds size limit"
        );
        return Ok(packed);
    }
    let mut decoder = GzDecoder::new(packed.as_slice());
    let mut plain = Vec::new();
    (&mut decoder)
        .take((MAX_DECOMPRESSED_BYTES as u64) + 1)
        .read_to_end(&mut plain)
        .context("invalid v2 gzip payload")?;
    ensure!(
        plain.len() <= MAX_DECOMPRESSED_BYTES,
        "decompressed payload exceeds size limit"
    );
    ensure!(
        decoder.into_inner().is_empty(),
        "trailing bytes in gzip payload"
    );
    Ok(plain)
}

fn secret_path(var: &str, basename: &str) -> Result<PathBuf> {
    if let Some(path) = env::var_os(var).filter(|value| !value.is_empty()) {
        return Ok(path.into());
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .context("home directory unavailable")?;
    Ok(PathBuf::from(home).join(".config/ai-digest").join(basename))
}

pub fn password_file_path() -> Result<PathBuf> {
    secret_path("DIGEST_SITE_PASSWORD_FILE", "site-password")
}
pub fn salt_file_path() -> Result<PathBuf> {
    secret_path("DIGEST_SITE_SALT_FILE", "site-salt")
}

fn read_secret_file(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_file(), "secret path must be a regular file");
    let mut file = fs::File::open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            file.metadata()?.permissions().mode() & 0o777 == 0o600,
            "secret file must have mode 0600"
        );
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .context("secret file is not readable UTF-8")?;
    Ok(contents)
}

pub fn write_secret_file(path: &Path, data: &str) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(data.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

pub fn read_password_file(path: &Path) -> Result<String> {
    let raw = read_secret_file(path)?;
    let password = raw
        .strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(&raw);
    ensure!(password.chars().count() >= 16, "password file is too short");
    Ok(password.into())
}

pub fn load_or_create_salt(path: &Path) -> Result<Vec<u8>> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let _lock = FileLock::acquire(Path::new(&lock_path))?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let raw = read_secret_file(path)?;
            let salt = hex::decode(raw.split_ascii_whitespace().collect::<String>())
                .context("invalid hex in salt file")?;
            ensure!(salt.len() == SALT_LEN, "invalid salt file length");
            Ok(salt)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut salt = vec![0; SALT_LEN];
            OsRng
                .try_fill_bytes(&mut salt)
                .map_err(|_| anyhow!("cannot generate encryption salt"))?;
            write_secret_file(path, &format!("{}\n", hex::encode(&salt)))?;
            Ok(salt)
        }
        Err(e) => Err(e.into()),
    }
}

pub fn inject_lock_ui_html(html: &str, lang: Lang) -> String {
    static TARGET: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?i)<button\b[^>]*class\s*=\s*["'][^"']*\btheme-btn\b[^"']*["'][^>]*>|</nav\s*>|</header\s*>|</body\s*>"#).unwrap()
    });
    static LEGACY_BUTTON: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<button\b[^>]*\bdata-lock\b[^>]*>.*?</button>").unwrap()
    });
    static TOP: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?is)<nav\b[^>]*class=["']issue-nav\s+top["'][^>]*>.*?</nav\s*>"#).unwrap()
    });
    static DOC_LANG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)<html\b[^>]*\blang=["']([^"']+)"#).unwrap());
    static PANE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)<div\s+class="lang-pane\s+lang-(zh|en)""#).unwrap());
    static BODY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</body\s*>").unwrap());

    let mut clean = ["digest-lock", "digest-lock-btn", "digest-lock-js"]
        .iter()
        .fold(html.to_owned(), |doc, mark| strip_marked(&doc, mark));
    clean = LEGACY_BUTTON.replace_all(&clean, "").into_owned();
    let lang = DOC_LANG
        .captures(&clean)
        .and_then(|m| i18n::normalize_lang(&m[1]).ok())
        .unwrap_or(lang);
    let panes: Vec<_> = PANE
        .captures_iter(&clean)
        .map(|m| {
            (
                m.get(0).unwrap().start(),
                if m[1].eq_ignore_ascii_case("en") {
                    Lang::En
                } else {
                    Lang::ZhCn
                },
            )
        })
        .collect();
    let mut positions = Vec::new();
    if panes.len() == 2 {
        let split = panes[1].0;
        for (fragment, offset, lg) in [
            (&clean[..split], 0, panes[0].1),
            (&clean[split..], split, panes[1].1),
        ] {
            if let Some(pos) = lock_button_pos(fragment, &TARGET, &TOP) {
                positions.push((pos + offset, lg));
            }
        }
    } else if let Some(pos) = lock_button_pos(&clean, &TARGET, &TOP) {
        positions.push((pos, lang));
    }
    for (pos, lg) in positions.into_iter().rev() {
        let button = format!(
            r#"<!-- digest-lock-btn:start --><button type="button" class="ctrl-btn lock-btn" data-lock aria-label="{}" title="{}">{}</button><!-- digest-lock-btn:end -->"#,
            esc(i18n::t(lg, "lock_title")),
            esc(i18n::t(lg, "lock_title")),
            esc(i18n::t(lg, "lock"))
        );
        clean.insert_str(pos, &button);
    }
    let script = format!(
        "<!-- digest-lock:start --><script>{}</script><!-- digest-lock:end -->",
        assets::LOCK_JS
    );
    if let Some(pos) = BODY.find(&clean).map(|m| m.start()) {
        clean.insert_str(pos, &script);
    } else {
        clean.push_str(&script);
    }
    clean
}

fn lock_button_pos(html: &str, target: &Regex, top: &Regex) -> Option<usize> {
    let mut fallback = None;
    for m in target.find_iter(html) {
        let text = m.as_str().to_ascii_lowercase();
        if text.starts_with("<button") {
            // Put the lock control after the theme button in the shared mast tools.
            return html[m.end()..]
                .to_ascii_lowercase()
                .find("</button>")
                .map(|i| m.end() + i + "</button>".len())
                .or(Some(m.start()));
        }
        if fallback.is_none() && text.starts_with("</header") {
            fallback = Some(m.start());
        }
        if fallback.is_none() && text.starts_with("</body") {
            fallback = Some(m.start());
        }
    }
    top.find(html)
        .and_then(|m| {
            m.as_str()
                .to_ascii_lowercase()
                .rfind("</nav")
                .map(|n| n + m.start())
        })
        .or(fallback)
}

#[derive(Clone, Copy, Default)]
pub struct UnlockOptions<'a> {
    pub lang: Lang,
    pub legacy_template: Option<&'a str>,
    pub lang_alt: Option<&'a LangAlt>,
    pub relative_page: &'a str,
    pub demo_password: Option<&'a str>,
}

fn lang_switch(options: UnlockOptions<'_>) -> String {
    let Some(alt) = options.lang_alt else {
        return String::new();
    };
    let rel = options.relative_page.trim_matches('/');
    let rel = rel.strip_suffix(".html").unwrap_or(rel);
    let rel = if rel == "index" {
        ""
    } else {
        rel.strip_suffix("/index").unwrap_or(rel)
    };
    let Some(href) = normalized_safe_url(&format!("{}{rel}", alt.base)) else {
        return String::new();
    };
    let lang = alt.lang;
    format!(
        r#"<a class="lang-alt" href="{}" hreflang="{lang}" lang="{lang}">{}</a>"#,
        esc(&href),
        esc(i18n::t(lang, "lang_name"))
    )
}

fn demo_hint(options: UnlockOptions<'_>) -> String {
    options.demo_password.map_or_else(String::new, |pass| {
        let label = esc(i18n::t(options.lang, "pw_demo_hint"))
            .replace("{pw}", &format!("<code>{}</code>", esc(pass)));
        format!(r#"<p class="demo-hint">{label}</p>"#)
    })
}

pub fn wrap_encrypted(
    plain: &[u8],
    key: &[u8; KEY_LEN],
    salt: &[u8],
    options: UnlockOptions<'_>,
) -> Result<String> {
    wrap_envelope(&encrypt_payload(plain, key, salt)?, options)
}

pub fn wrap_envelope(envelope: &Envelope, options: UnlockOptions<'_>) -> Result<String> {
    let envelope = escape_json_for_script(&serde_json::to_string(envelope)?);
    let localized = if let Some(template) = options.legacy_template {
        ensure!(
            template.matches(ENVELOPE_SLOT).count() == 1,
            "password template requires exactly one envelope placeholder"
        );
        localize_legacy_template(template, options)?
    } else {
        builtin_unlock(options)?
    };
    ensure!(
        localized.matches(ENVELOPE_SLOT).count() == 1,
        "password template requires exactly one localized envelope placeholder"
    );
    let shell = localized.replacen(ENVELOPE_SLOT, &envelope, 1);
    assert_encrypted_shell(&shell, options.lang)?;
    Ok(shell)
}

pub fn localize_legacy_template(template: &str, options: UnlockOptions<'_>) -> Result<String> {
    static PLACEHOLDERS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\{\{(js:)?([A-Za-z0-9_]+)\}\}").unwrap());
    let mut out = String::new();
    let mut pos = 0;
    for m in PLACEHOLDERS.captures_iter(template) {
        let whole = m.get(0).unwrap();
        out.push_str(&template[pos..whole.start()]);
        let js = m.get(1).is_some();
        let key = &m[2];
        let value = match (js, key) {
            (false, "lang_switch") => lang_switch(options),
            (false, "demo_hint") => demo_hint(options),
            _ => {
                let value = i18n::string(options.lang, key)
                    .with_context(|| format!("unknown password template translation {key}"))?;
                if js {
                    escape_json_for_script(&serde_json::to_string(value)?)
                } else {
                    esc(value)
                }
            }
        };
        out.push_str(&value);
        pos = whole.end();
    }
    out.push_str(&template[pos..]);
    Ok(out)
}

pub fn unlock_config_script(options: UnlockOptions<'_>) -> Result<String> {
    let mut config = serde_json::Map::new();
    for key in [
        "pw_title",
        "pw_label",
        "pw_open",
        "pw_wrong",
        "pw_remember",
        "pw_opening",
        "pw_unsupported",
    ] {
        config.insert(
            key.into(),
            serde_json::Value::from(i18n::t(options.lang, key)),
        );
    }
    config.insert(
        "lang".into(),
        serde_json::Value::from(options.lang.as_str()),
    );
    Ok(escape_json_for_script(&serde_json::to_string(&config)?))
}

fn builtin_unlock(options: UnlockOptions<'_>) -> Result<String> {
    let lang = options.lang;
    let t = |key| esc(i18n::t(lang, key));
    let config = unlock_config_script(options)?;
    let alt = lang_switch(options);
    let alt = if alt.is_empty() {
        alt
    } else {
        format!(r#"<p class="lang-alt">{alt}</p>"#)
    };
    Ok(format!(
        r##"<!doctype html>
<html lang="{lang}"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="theme-color" content="#121110">
<meta name="color-scheme" content="dark">
<script>{boot}</script>
<title>{title}</title>
<link rel="icon" href="/favicon.svg" type="image/svg+xml">
<meta name="robots" content="noindex,nofollow">
<!-- digest-geo-head:start --><!-- digest-geo-head:end -->
<style>{css}</style>
</head>
<body>
<main id="box"><p class="np"><span class="ai">AI</span> Digest</p><hr>
<p class="kicker">{kicker}</p><h1 id="pwTitle">{heading}</h1>
<noscript><p>{noscript}</p></noscript>
<form id="pwForm" novalidate>
<label for="pw">{label}</label><input id="pw" type="password" autocomplete="current-password" required>
<label class="rm-row"><input id="rm" type="checkbox"><span>{remember}</span></label>
<button id="go" type="submit"><span class="go-lbl">{open}</span><span class="spin" hidden aria-hidden="true"></span></button>
<p class="err" role="alert" aria-live="assertive"></p></form>
{hint}{alt}</main>
<script type="application/json" id="c">{ENVELOPE_SLOT}</script>
<script type="application/json" id="cfg">{config}</script>
<script>{unlock}</script>
</body></html>
"##,
        boot = assets::BOOT_JS,
        css = assets::UNLOCK_CSS,
        unlock = assets::UNLOCK_JS,
        title = t("pw_title"),
        kicker = lang.choose("Encrypted Edition", "加密刊"),
        heading = lang.choose("Enter passphrase to read", "输入口令阅读本期"),
        label = t("pw_label"),
        remember = t("pw_remember"),
        open = t("pw_open"),
        noscript = t("pw_noscript"),
        hint = demo_hint(options)
    ))
}

fn attr<'a>(tag: &'a html5gum::StartTag, name: &[u8]) -> &'a [u8] {
    tag.attributes
        .get(name)
        .map_or(b"", |value| value.0.as_slice())
}

#[derive(Debug)]
struct Script {
    body: String,
    executable: bool,
    speculation_rules: bool,
    envelope: bool,
}

fn scripts(html: &str) -> Result<Vec<Script>> {
    let mut out = Vec::new();
    let mut active: Option<Script> = None;
    for token in html_tokens(html) {
        match token {
            Token::StartTag(tag) if tag.name.0 == b"script" => {
                ensure!(active.is_none(), "nested script element");
                let kind = String::from_utf8_lossy(attr(&tag, b"type"))
                    .trim()
                    .to_ascii_lowercase();
                let external = tag.attributes.contains_key(b"src".as_slice());
                active = Some(Script {
                    body: String::new(),
                    executable: !external && !DATA_SCRIPT_TYPES.contains(&kind.as_bytes()),
                    speculation_rules: !external && kind == "speculationrules",
                    envelope: !external && kind == "application/json" && attr(&tag, b"id") == b"c",
                });
            }
            Token::String(s) => {
                if let Some(active) = &mut active {
                    active.body.push_str(std::str::from_utf8(&s.0)?);
                }
            }
            Token::EndTag(tag) if tag.name.0 == b"script" => {
                if let Some(script) = active.take() {
                    out.push(script);
                }
            }
            _ => {}
        }
    }
    ensure!(active.is_none(), "unclosed script element");
    Ok(out)
}

pub fn extract_envelope(html: &str) -> Result<Envelope> {
    let mut found = scripts(html)?.into_iter().filter(|script| script.envelope);
    let script = found.next().context("missing encrypted envelope")?;
    ensure!(found.next().is_none(), "duplicate encrypted envelope");
    ensure!(script.body.contains("\"ct\":"), "missing ciphertext");
    let envelope = serde_json::from_str(&script.body)?;
    check_envelope(&envelope)?;
    Ok(envelope)
}

fn normalize_lines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn script_hash(script: &str) -> String {
    format!(
        "'sha256-{}'",
        STANDARD.encode(Sha256::digest(normalize_lines(script).as_bytes()))
    )
}

// Trusted inline script hashes from previous v1.0.0 and v1.0.1 releases so incremental
// `publish --issue` / `--rebuild` upgrades succeed without forcing --rerender-all.
const KNOWN_PREVIOUS_RELEASE_HASHES: &[&str] = &[
    "'sha256-K8AMXq9YtQjRNhY29wqFrBeVSAmso/fUzqfuqELyxS0='", // v1.0.0 boot.js
    "'sha256-za2UDgPTC4kgNKKeVfbNEujaIgnNHJnzdVgff5jcG5o='", // v1.0.0 reader.js
    "'sha256-2d1HlyBoJ0RTWBo1V8cJq5gqndqRo/+j5jGqg+hN+Vk='", // v1.0.0 index.js
    "'sha256-ULWcGE48zLdDVllI0EYBwq+iTR3kAcDbzv+3QVUarYQ='", // v1.0.1 boot.js
    "'sha256-2Ivlf0XJIcWpuKux6MsYqWgB57xkKsnxmYqoT9r6D4g='", // v1.0.1 reader.js
    "'sha256-aXNfRvBaL19OMemVpb9AUUGmEbb1BwKS6xDxb4Pb5HU='", // v1.0.1 index.js
    "'sha256-9HqZwpsqf7FMp1SP40jMRJ6HOhL5ezGD/GcJa+n99Qw='", // v1.0.0/v1.0.1 unlock.js
    "'sha256-bAOgKqYwVOp7VK1g8OY7p0/ZwTM83IkvpUD6b66i2Qw='", // v1.0.0/v1.0.1 lock.js
];

#[derive(Clone, Debug)]
pub struct ScriptAllowlist {
    hashes: BTreeSet<String>,
}

impl Default for ScriptAllowlist {
    fn default() -> Self {
        let mut hashes: BTreeSet<String> =
            EXECUTABLE_ASSETS.iter().map(|s| script_hash(s)).collect();
        hashes.extend(
            KNOWN_PREVIOUS_RELEASE_HASHES
                .iter()
                .map(|h| (*h).to_owned()),
        );
        Self { hashes }
    }
}

impl ScriptAllowlist {
    pub fn contains_hash(&self, hash: &str) -> bool {
        self.hashes.contains(hash)
    }

    pub(crate) fn allow_legacy_hash(&mut self, hash: &str) {
        self.hashes.insert(hash.to_owned());
    }

    /// Add exact script bodies from a separately trusted legacy template or release.
    pub fn allow_legacy_scripts<'a>(&mut self, trusted_scripts: impl IntoIterator<Item = &'a str>) {
        self.hashes
            .extend(trusted_scripts.into_iter().map(script_hash));
    }

    /// Use only with a trusted template, never with a page awaiting validation.
    pub fn allow_legacy_template(
        &mut self,
        trusted_template: &str,
        options: UnlockOptions<'_>,
    ) -> Result<()> {
        let localized = localize_legacy_template(trusted_template, options)?;
        let scripts = scripts(&localized)?;
        self.allow_legacy_scripts(
            scripts
                .iter()
                .filter(|s| s.executable)
                .map(|s| s.body.as_str()),
        );
        Ok(())
    }
}

pub fn collect_script_hashes(html: &str, allowlist: &ScriptAllowlist) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    let mut inline_bytes = 0;
    for script in scripts(html)?.into_iter().filter(|s| s.executable) {
        inline_bytes += script.body.len();
        let hash = script_hash(&script.body);
        ensure!(
            allowlist.hashes.contains(&hash),
            "unexpected executable inline script"
        );
        hashes.push(hash);
    }
    ensure!(
        inline_bytes <= MAX_INLINE_JS_BYTES,
        "inline JavaScript exceeds the 30 KiB page budget"
    );
    Ok(hashes)
}

const COMPANION_HEADERS: &[(&str, &str)] = &[
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("X-Frame-Options", "DENY"),
    (
        "Strict-Transport-Security",
        "max-age=31536000; includeSubDomains",
    ),
    ("Cross-Origin-Opener-Policy", "same-origin"),
    ("Cross-Origin-Resource-Policy", "same-origin"),
    (
        "Permissions-Policy",
        "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
    ),
];

fn collect_speculation_rules_hashes(html: &str) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    for script in scripts(html)?.into_iter().filter(|s| s.speculation_rules) {
        ensure!(
            normalize_lines(&script.body) == crate::model::SPECULATION_RULES_JSON,
            "unexpected inline speculationrules script"
        );
        hashes.push(script_hash(&script.body));
    }
    Ok(hashes)
}

pub fn expected_security_headers(
    public: &Path,
    plain: &Path,
    public_indexable: bool,
    allowlist: &ScriptAllowlist,
) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeSet::new();
    for path in html_files(plain)?.into_iter().chain(html_files(public)?) {
        let html = fs::read_to_string(&path)?;
        hashes.extend(collect_script_hashes(&html, allowlist).with_context(|| {
            format!(
                "CSP validation failed for {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        })?);
        hashes.extend(collect_speculation_rules_hashes(&html).with_context(|| {
            format!(
                "CSP speculationrules validation failed for {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )
        })?);
    }
    let script_src = if hashes.is_empty() {
        "'none'".to_owned()
    } else {
        hashes.into_iter().collect::<Vec<_>>().join(" ")
    };
    let policy = format!(
        "default-src 'none'; script-src {script_src}; script-src-attr 'none'; style-src 'unsafe-inline'; img-src 'self'; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; object-src 'none'; upgrade-insecure-requests"
    );
    let mut expected = BTreeMap::from([("Content-Security-Policy".to_owned(), policy)]);
    expected.extend(
        COMPANION_HEADERS
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into())),
    );
    if !public_indexable {
        expected.insert("X-Robots-Tag".into(), "noindex, nofollow".into());
    }
    Ok(expected)
}

fn parse_csp_directives(policy: &str) -> Result<BTreeMap<String, Vec<String>>> {
    let mut directives = BTreeMap::new();
    for raw in policy.split(';') {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        let mut tokens = part.split_whitespace();
        let name = tokens
            .next()
            .context("empty CSP directive")?
            .to_ascii_lowercase();
        let values: Vec<String> = tokens.map(str::to_owned).collect();
        ensure!(
            directives.insert(name.clone(), values).is_none(),
            "duplicate CSP directive: {name}"
        );
    }
    ensure!(!directives.is_empty(), "empty Content-Security-Policy");
    Ok(directives)
}

pub fn assert_csp_policy_matches(actual: &str, expected: &str) -> Result<()> {
    let actual_dirs = parse_csp_directives(actual)?;
    let expected_dirs = parse_csp_directives(expected)?;
    ensure!(
        !actual_dirs.contains_key("script-src-elem"),
        "unexpected CSP script-src-elem override"
    );
    ensure!(
        actual_dirs.get("script-src-attr").map(Vec::as_slice) == Some(&["'none'".to_owned()][..]),
        "CSP must set script-src-attr 'none'"
    );
    ensure!(
        actual_dirs.len() == expected_dirs.len(),
        "CSP directive set does not match expected policy"
    );
    for (name, want) in &expected_dirs {
        let got = actual_dirs
            .get(name)
            .with_context(|| format!("missing CSP directive: {name}"))?;
        if name == "script-src" {
            let got_set: BTreeSet<&str> = got.iter().map(String::as_str).collect();
            let want_set: BTreeSet<&str> = want.iter().map(String::as_str).collect();
            ensure!(
                got.len() == got_set.len() && got_set == want_set,
                "CSP script-src does not match expected trusted hashes"
            );
        } else {
            ensure!(
                got == want,
                "CSP directive {name} does not match expected policy"
            );
        }
    }
    Ok(())
}

pub fn finalize_csp_headers(
    public: &Path,
    plain: &Path,
    public_indexable: bool,
    allowlist: &ScriptAllowlist,
) -> Result<BTreeMap<String, String>> {
    let mut new = expected_security_headers(public, plain, public_indexable, allowlist)?;
    let path = public.join("_headers");
    let old = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut kept = String::new();
    let mut route = "";
    for line in old.lines() {
        let trimmed = line.trim();
        if line.trim_start() == line
            && (line.starts_with('/')
                || line.starts_with("http://")
                || line.starts_with("https://"))
        {
            route = trimmed;
        }
        if trimmed == "/*" {
            continue;
        }
        if trimmed.starts_with('!') {
            if route == "/*" {
                bail!("global header removals cannot override finalized security headers");
            }
            let target = parse_header_removal_target(trimmed)
                .context("malformed header removal directive in _headers")?;
            ensure!(
                !is_protected_security_header(target, !public_indexable),
                "route-level removal of security header is forbidden: {target}"
            );
        }
        if route == "/*" {
            if let Some((key, value)) = trimmed.split_once(':') {
                let owned = new.keys().any(|k| k.eq_ignore_ascii_case(key))
                    || key.eq_ignore_ascii_case("X-Robots-Tag");
                if owned {
                    continue;
                }
                new.insert(key.into(), value.trim().into());
                continue;
            }
        }
        kept.push_str(line);
        kept.push('\n');
    }
    let mut text = String::from("/*\n");
    for (key, value) in &new {
        text.push_str(&format!("  {key}: {value}\n"));
    }
    if !old.lines().any(|line| line.trim() == "/favicon.svg") {
        text.push_str("/favicon.svg\n  Cache-Control: public, max-age=86400\n");
    }
    text.push_str(&kept);
    atomic_write_text(&path, &text)?;
    Ok(new)
}

pub const PRIVACY_PATTERNS: &[(&str, &str)] = &[
    (r"[\w.+-]+@[\w.-]+\.\w+", "email"),
    (r"我(在)?(X|Twitter).{0,20}(关注|收藏|书签)", "x-circle"),
    (r"(X|Twitter).{0,8}圈子", "x-circle"),
    (r"我关注的人", "x-circle"),
    (r"我的(关注|收藏|书签)", "x-circle"),
    (
        r"\bI (follow|bookmarked|saved)\b.{0,30}\b(on )?(X|Twitter)\b",
        "x-circle",
    ),
    (r"\b(people|accounts) I follow\b", "x-circle"),
    (
        r"\bmy (follows|bookmarks|likes|following list|X circle|Twitter circle)\b",
        "x-circle",
    ),
    (r"\b(your|my) (X|Twitter) (circle|following)\b", "x-circle"),
    (
        r"(?:/[U]sers/|/[h]ome/|C:\\[U]sers\\)[A-Za-z0-9._-]+",
        "local-path",
    ),
    (
        r"\b(?:ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_-]{35}|AKIA[0-9A-Z]{16}|sk-[A-Za-z0-9_-]{24,})\b",
        "token-secret",
    ),
    (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "private-key"),
];

pub fn iter_plain_files(plain: &Path) -> Result<Vec<PathBuf>> {
    Ok(files_under(plain)?)
}

pub fn iter_unencrypted_public(public: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for path in files_under(public)? {
        let rel = path.strip_prefix(public)?;
        if path.extension().is_some_and(|x| x == "html") {
            let page = fs::read_to_string(&path)?;
            if !is_envelope_document(&page) {
                paths.push(path);
            }
        } else if rel.starts_with(".well-known")
            || path
                .extension()
                .is_some_and(|x| ["md", "txt", "xml", "json", "svg"].iter().any(|e| x == *e))
            || matches!(rel.to_str(), Some("_headers" | "_redirects" | "_worker.js"))
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn is_envelope_document(html: &str) -> bool {
    has_password_prompt(html) && extract_envelope(html).is_ok()
}

fn iter_public_scannable(public: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for path in files_under(public)? {
        let rel = path.strip_prefix(public)?;
        if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("html"))
            || rel.starts_with(".well-known")
            || path
                .extension()
                .is_some_and(|x| ["md", "txt", "xml", "json", "svg"].iter().any(|e| x == *e))
            || matches!(rel.to_str(), Some("_headers" | "_redirects" | "_worker.js"))
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn strip_envelope_script_body(html: &str) -> String {
    let normalized = normalize_lines(html);
    if !is_envelope_document(&normalized) {
        return normalized;
    }
    static CLOSE_SCRIPT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)</script\s*>").unwrap());
    let bytes = normalized.as_bytes();
    let mut out = String::with_capacity(normalized.len());
    let mut i = 0;
    let mut stripped = false;
    while i < bytes.len() {
        if normalized[i..].starts_with("<!--") {
            if let Some(end) = normalized[i + 4..].find("-->") {
                let next = i + 4 + end + 3;
                out.push_str(&normalized[i..next]);
                i = next;
                continue;
            }
        }
        if !stripped && bytes[i] == b'<' {
            let rest = &normalized[i..];
            if rest
                .as_bytes()
                .get(..7)
                .is_some_and(|b| b.eq_ignore_ascii_case(b"<script"))
                && rest
                    .as_bytes()
                    .get(7)
                    .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>')
            {
                if let Some(tag_end_rel) = rest.find('>') {
                    let open_tag = &rest[..=tag_end_rel];
                    let after_open = i + tag_end_rel + 1;
                    let is_env_tag = html_tokens(open_tag).next().is_some_and(|tok| {
                        if let Token::StartTag(tag) = tok {
                            let kind = String::from_utf8_lossy(attr(&tag, b"type"))
                                .trim()
                                .to_ascii_lowercase();
                            let external = tag.attributes.contains_key(b"src".as_slice());
                            tag.name.0 == b"script"
                                && !external
                                && kind == "application/json"
                                && attr(&tag, b"id") == b"c"
                        } else {
                            false
                        }
                    });
                    if is_env_tag {
                        if let Some(close) = CLOSE_SCRIPT.find(&normalized[after_open..]) {
                            let body = &normalized[after_open..after_open + close.start()];
                            if serde_json::from_str::<Envelope>(body)
                                .ok()
                                .is_some_and(|env| check_envelope(&env).is_ok())
                            {
                                out.push_str(open_tag);
                                out.push_str(
                                    &normalized
                                        [after_open + close.start()..after_open + close.end()],
                                );
                                i = after_open + close.end();
                                stripped = true;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        let ch = normalized[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn privacy_scan(site: &Path, plain: &Path, public: &Path) -> Result<()> {
    let configured = env::var_os("DIGEST_PRIVACY_PATTERNS_FILE")
        .filter(|s| !s.to_string_lossy().trim().is_empty())
        .map(PathBuf::from);
    privacy_scan_with_patterns(site, plain, public, configured.as_deref())
}

pub fn privacy_scan_with_patterns(
    site: &Path,
    plain: &Path,
    public: &Path,
    override_file: Option<&Path>,
) -> Result<()> {
    let default = site.join("privacy-patterns.txt");
    let path = override_file.unwrap_or(&default);
    let custom = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound && override_file.is_none() => String::new(),
        Err(e) => return Err(e).context("cannot read privacy patterns"),
    };
    let extra = custom
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| {
            let (pattern, label) = line.split_once('\t').unwrap_or((line, ""));
            (
                pattern.trim(),
                if label.trim().is_empty() {
                    "custom"
                } else {
                    label.trim()
                },
            )
        });
    let mut patterns = Vec::new();
    for (p, label) in PRIVACY_PATTERNS.iter().copied().chain(extra) {
        let pattern = RegexBuilder::new(p)
            .case_insensitive(true)
            .dot_matches_new_line(true)
            .build()
            .map_err(|_| anyhow!("invalid privacy pattern ({label})"))?;
        patterns.push((pattern, label));
    }
    let plain_targets = iter_plain_files(plain)?
        .into_iter()
        .map(|path| (path, false));
    let public_targets = iter_public_scannable(public)?
        .into_iter()
        .map(|path| (path, true));
    for (path, is_public) in plain_targets.chain(public_targets) {
        let raw =
            fs::read_to_string(&path).context("privacy scan could not read a UTF-8 output file")?;
        let scannable = if is_public
            && path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("html"))
        {
            strip_envelope_script_body(&raw)
        } else {
            raw
        };
        let visible = visible_text(&scannable).replace('\u{2011}', "-");
        let block_text = join_visible_text(&scannable, true).replace('\u{2011}', "-");
        let normalized_raw = scannable.replace('\u{2011}', "-");
        for (pattern, label) in &patterns {
            ensure!(
                !pattern.is_match(&normalized_raw)
                    && !pattern.is_match(&visible)
                    && !pattern.is_match(&block_text),
                "privacy scan failed ({label})"
            );
        }
    }
    Ok(())
}

fn visible_text(html: &str) -> String {
    join_visible_text(html, false)
}

fn join_visible_text(html: &str, separate_blocks: bool) -> String {
    let mut out = String::new();
    let mut skip = None;
    for token in html_tokens(html) {
        match token {
            Token::StartTag(tag)
                if skip.is_none()
                    && [b"script".as_slice(), b"style"].contains(&tag.name.0.as_slice()) =>
            {
                skip = Some(tag.name);
            }
            Token::EndTag(tag) if skip.as_ref() == Some(&tag.name) => skip = None,
            Token::String(s) if skip.is_none() => out.push_str(&String::from_utf8_lossy(&s.0)),
            Token::StartTag(tag)
                if separate_blocks
                    && skip.is_none()
                    && matches!(
                        tag.name.0.as_slice(),
                        b"article" | b"div" | b"p" | b"section" | b"li" | b"br"
                    ) =>
            {
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

pub fn assert_encrypted_shell(html: &str, lang: Lang) -> Result<()> {
    ensure!(
        !html.contains(ENVELOPE_SLOT),
        "unreplaced password template placeholder"
    );
    let envelope = extract_envelope(html)?;
    ensure!(!envelope.ct.is_empty(), "missing encrypted ciphertext");
    let visible = visible_text(html);
    for key in ["pw_title", "pw_label", "pw_open"] {
        ensure!(
            visible.contains(i18n::t(lang, key)),
            "missing visible password prompt ({key})"
        );
    }
    let mut has = [false; 3];
    for token in html_tokens(html) {
        let Token::StartTag(tag) = token else {
            continue;
        };
        match tag.name.0.as_slice() {
            b"input" => {
                has[0] |= attr(&tag, b"id") == b"pw"
                    && attr(&tag, b"type").eq_ignore_ascii_case(b"password");
                has[1] |= attr(&tag, b"id") == b"rm"
                    && attr(&tag, b"type").eq_ignore_ascii_case(b"checkbox");
            }
            b"meta" if attr(&tag, b"name").eq_ignore_ascii_case(b"robots") => {
                has[2] |= String::from_utf8_lossy(attr(&tag, b"content"))
                    .split(',')
                    .any(|val| val.trim().eq_ignore_ascii_case("noindex"));
            }
            _ => {}
        }
    }
    ensure!(
        has.into_iter().all(|value| value),
        "missing password inputs or noindex directive"
    );
    Ok(())
}

pub fn matches_shell_template(shell: &str, localized_template: &str) -> bool {
    let Some((before, after)) = localized_template.split_once(ENVELOPE_SLOT) else {
        return false;
    };
    if after.contains(ENVELOPE_SLOT) {
        return false;
    }
    let Some(envelope_json) = shell
        .strip_prefix(before)
        .and_then(|s| s.strip_suffix(after))
    else {
        return false;
    };
    serde_json::from_str::<Envelope>(envelope_json)
        .ok()
        .is_some_and(|env| check_envelope(&env).is_ok())
}

pub fn assert_no_plaintext_leak(
    plain: &str,
    wrapped: &str,
    trusted_shell: &str,
    lang: Lang,
) -> Result<()> {
    assert_encrypted_shell(wrapped, lang)?;
    static HEADINGS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<h[1-6]\b[^>]*>(.*?)</h[1-6]\s*>").unwrap());
    static SNIPPETS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<(?:p|li)\b[^>]*>(.*?)</(?:p|li)\s*>").unwrap());
    let shell_without_ct = strip_envelope_script_body(wrapped);
    let visible = clean_text(&visible_text(&shell_without_ct));
    let trusted = clean_text(&visible_text(trusted_shell));
    for m in HEADINGS.captures_iter(plain) {
        let snippet = clean_text(&visible_text(&m[1]));
        if snippet.chars().count() >= 8 && !trusted.contains(&snippet) {
            ensure!(
                !visible.contains(&snippet)
                    && !shell_without_ct.contains(&snippet)
                    && !shell_without_ct.contains(&esc(&snippet)),
                "plaintext heading leaked into encrypted shell"
            );
        }
    }
    for m in SNIPPETS.captures_iter(plain) {
        let snippet = clean_text(&visible_text(&m[1]));
        if snippet.chars().count() >= 16 && !trusted.contains(&snippet) {
            ensure!(
                !visible.contains(&snippet)
                    && !shell_without_ct.contains(&snippet)
                    && !shell_without_ct.contains(&esc(&snippet)),
                "plaintext content leaked into encrypted shell"
            );
        }
    }
    Ok(())
}

pub type EncryptionCache = BTreeMap<String, String>;

pub fn enc_fingerprint(
    plain: &[u8],
    salt: &[u8],
    key: &[u8; KEY_LEN],
    template: &str,
    alternate: &str,
    lang: Lang,
) -> String {
    let hash = |data: &[u8]| hex::encode(Sha256::digest(data));
    format!(
        "{}:{}:{}:{}:{}:{lang}",
        hash(plain),
        &hash(salt)[..16],
        &hash(key)[..16],
        &hash(template.as_bytes())[..16],
        &hash(alternate.as_bytes())[..8]
    )
}

pub fn load_enc_cache(path: &Path) -> Result<EncryptionCache> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(serde_json::from_str(&s).unwrap_or_default()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(EncryptionCache::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn save_enc_cache(path: &Path, cache: &EncryptionCache) -> Result<()> {
    atomic_write_text(path, &serde_json::to_string_pretty(cache)?)?;
    Ok(())
}

fn html_files(root: &Path) -> Result<Vec<PathBuf>> {
    Ok(files_under(root)?
        .into_iter()
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("html"))
        })
        .collect())
}

fn shell_lang(rel: &Path, default: Lang) -> Lang {
    if rel.starts_with("en") {
        Lang::En
    } else if rel.starts_with("zh") || rel.starts_with("zh-CN") {
        Lang::ZhCn
    } else {
        default
    }
}

fn control_page(relative: &Path) -> bool {
    relative == Path::new("404.html")
}

pub fn assert_public_matches_plain(plain: &Path, public: &Path) -> Result<usize> {
    let pages = matching_pages(plain, public)?;
    for (source, dest, _) in &pages {
        let public_bytes = fs::read(dest)?;
        let html = std::str::from_utf8(&public_bytes)?;
        ensure!(
            !has_password_prompt(html) && !html.contains(ENVELOPE_SLOT),
            "password prompt in public plaintext"
        );
        ensure!(
            public_bytes == fs::read(source)?,
            "published HTML differs from plaintext"
        );
    }
    Ok(pages.len())
}

fn has_password_prompt(html: &str) -> bool {
    html_tokens(html).any(|token| matches!(token, Token::StartTag(tag) if attr(&tag, b"id") == b"rm" || attr(&tag, b"id") == b"pw"))
}

pub fn render_unlock_template(options: UnlockOptions<'_>) -> Result<String> {
    match options.legacy_template {
        Some(tmpl) => localize_legacy_template(tmpl, options),
        None => builtin_unlock(options),
    }
}

pub fn assert_public_is_encrypted(
    plain: &Path,
    public: &Path,
    slug: &str,
    lang: Lang,
) -> Result<usize> {
    assert_public_is_encrypted_with(plain, public, slug, |rel| {
        let lang = shell_lang(rel.strip_prefix(slug).unwrap_or(rel), lang);
        Ok(UnlockOptions {
            lang,
            ..UnlockOptions::default()
        })
    })
}

pub fn assert_public_is_encrypted_with<'a>(
    plain: &Path,
    public: &Path,
    slug: &str,
    options_for: impl Fn(&Path) -> Result<UnlockOptions<'a>>,
) -> Result<usize> {
    validate_slug(slug)?;
    let pages = matching_pages(plain, public)?;
    for (source, dest, rel) in &pages {
        let options = options_for(rel)?;
        let html = fs::read_to_string(dest)?;
        let trusted = render_unlock_template(options)?;
        assert_no_plaintext_leak(&fs::read_to_string(source)?, &html, &trusted, options.lang)?;
        ensure!(
            matches_shell_template(&html, &trusted),
            "encrypted shell does not match expected unlock template: {}",
            rel.display()
        );
    }
    Ok(pages.len())
}

fn matching_pages(plain: &Path, public: &Path) -> Result<Vec<(PathBuf, PathBuf, PathBuf)>> {
    let mut pages = Vec::new();
    for dest in html_files(public)? {
        let rel = dest.strip_prefix(public)?.to_owned();
        if control_page(&rel) {
            continue;
        }
        let source = plain.join(&rel);
        ensure!(
            source.is_file(),
            "published page is missing its plaintext source"
        );
        pages.push((source, dest, rel));
    }
    ensure!(!pages.is_empty(), "no published HTML to verify");
    for source in html_files(plain)? {
        let rel = source.strip_prefix(plain)?;
        ensure!(
            control_page(rel) || public.join(rel).is_file(),
            "plaintext source is missing its published page"
        );
    }
    Ok(pages)
}

/// Work on the final plain tree after navigation injection. GEO and CSP finalization are
/// separate steps so the publisher can write response headers after all scripts are present.
pub fn encrypt_all_pages(
    paths: &DigestPaths,
    key: &[u8; KEY_LEN],
    salt: &[u8],
    options: UnlockOptions<'_>,
) -> Result<Vec<PathBuf>> {
    let languages = |rel: &Path| UnlockOptions {
        lang: shell_lang(rel, options.lang),
        relative_page: "",
        ..options
    };
    encrypt_all_pages_with(paths, key, salt, languages)
}

pub(crate) fn encrypt_all_pages_with<'a>(
    paths: &DigestPaths,
    key: &[u8; KEY_LEN],
    salt: &[u8],
    options_for: impl Fn(&Path) -> UnlockOptions<'a>,
) -> Result<Vec<PathBuf>> {
    let mut fallback_template = None;
    let pages = html_files(&paths.plain)?;
    ensure!(
        pages.iter().any(|path| path
            .strip_prefix(&paths.plain)
            .is_ok_and(|rel| !control_page(rel))),
        "no plaintext HTML to encrypt"
    );
    privacy_scan(&paths.site, &paths.plain, &paths.public)?;
    let old = load_enc_cache(&paths.enc_cache)?;
    let mut cache = EncryptionCache::new();
    let mut changed = Vec::new();
    for path in pages {
        let rel = path.strip_prefix(&paths.plain)?;
        if control_page(rel) {
            continue;
        }
        let name = rel.to_str().context("invalid HTML path")?;
        let options = options_for(rel);
        let lang = options.lang;
        if options.legacy_template.is_none() && fallback_template.is_none() {
            fallback_template = Some(
                match fs::read_to_string(paths.site.join("password_template.html")) {
                    Ok(template) => Some(template),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                },
            );
        }
        let options = UnlockOptions {
            legacy_template: options
                .legacy_template
                .or_else(|| fallback_template.as_ref()?.as_deref()),
            relative_page: if options.relative_page.is_empty() {
                name
            } else {
                options.relative_page
            },
            ..options
        };
        let source = fs::read_to_string(&path)?;
        let plain = inject_lock_ui_html(&source, lang);
        let tmpl = match options.legacy_template {
            Some(tmpl) => localize_legacy_template(tmpl, options)?,
            None => builtin_unlock(options)?,
        };
        let fingerprint = enc_fingerprint(
            plain.as_bytes(),
            salt,
            key,
            &tmpl,
            &lang_switch(options),
            lang,
        );
        let dest = paths.public.join(rel);
        if source != plain {
            atomic_write_text(&path, &plain)?;
        }
        let reused = old.get(name) == Some(&fingerprint)
            && fs::read_to_string(&dest).is_ok_and(|previous| {
                matches_shell_template(&previous, &tmpl)
                    && assert_no_plaintext_leak(&plain, &previous, &tmpl, lang).is_ok()
                    && extract_envelope(&previous)
                        .and_then(|env| decrypt_envelope_with_key(&env, key))
                        .is_ok_and(|bytes| bytes == plain.as_bytes())
            });
        if !reused {
            let wrapped = wrap_encrypted(plain.as_bytes(), key, salt, options)?;
            assert_no_plaintext_leak(&plain, &wrapped, &tmpl, lang)?;
            atomic_write_text(&dest, &wrapped)?;
            changed.push(rel.to_owned());
        }
        cache.insert(name.into(), fingerprint);
    }
    for dest in html_files(&paths.public)? {
        let relative = dest.strip_prefix(&paths.public)?;
        if !control_page(relative) && !paths.plain.join(relative).is_file() {
            fs::remove_file(&dest)?;
            changed.push(relative.to_owned());
        }
    }
    save_enc_cache(&paths.enc_cache, &cache)?;
    Ok(changed)
}
