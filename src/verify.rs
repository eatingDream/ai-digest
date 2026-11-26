use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use clap::Args;
use html5gum::Token;
use url::Url;

use crate::crypto::{self, KEY_LEN, MAX_DECOMPRESSED_BYTES, PBKDF2_ITERATIONS};
use crate::model::{files_under, html_tokens, DigestPaths};
use crate::net::{read_bytes_limited, redact_url, SafeHttpClient};

#[derive(Args, Debug)]
pub struct VerifyArgs {
    #[arg(long, help = "Compare the local public and plaintext trees")]
    pub local: bool,
    #[arg(long, value_name = "URL", help = "Verify extensionless deployed URLs")]
    pub url_base: Option<Url>,
    #[arg(long = "url", value_name = "URL", requires = "plains")]
    pub urls: Vec<Url>,
    #[arg(long = "plain", value_name = "PATH", requires = "urls")]
    pub plains: Vec<PathBuf>,
    #[arg(long, value_name = "PATH")]
    pub password_file: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
#[error("no verification targets under site/plain or supplied on the command line")]
pub struct NoTargets;

#[derive(Debug, thiserror::Error)]
#[error("--url and --plain must be paired")]
pub struct UnpairedUrls;

#[derive(Debug)]
enum Published {
    File(PathBuf),
    Url(Url),
}

impl std::fmt::Display for Published {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(path) => path.display().fmt(f),
            Self::Url(url) => write!(f, "GET {}", redact_url(url)),
        }
    }
}

#[derive(Debug)]
struct Target {
    published: Published,
    plaintext: PathBuf,
    control: bool,
}

fn html_paths(dir: &Path) -> Result<BTreeSet<PathBuf>> {
    Ok(files_under(dir)?
        .into_iter()
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("html"))
        })
        .map(|p| p.strip_prefix(dir).map(Path::to_owned))
        .collect::<std::result::Result<_, _>>()?)
}

fn discover(paths: &DigestPaths, local: bool) -> Result<BTreeSet<PathBuf>> {
    let mut rels = html_paths(&paths.plain)?;
    if rels.iter().all(|p| p == Path::new("404.html")) {
        return Err(NoTargets.into());
    }
    if local {
        rels.extend(
            html_paths(&paths.public)?
                .into_iter()
                .filter(|p| p != Path::new("404.html")),
        );
    }
    Ok(rels)
}

pub fn extensionless(relative: &str) -> &str {
    if relative == "index.html" {
        ""
    } else if let Some(p) = relative.strip_suffix("/index.html") {
        &relative[..p.len() + 1]
    } else {
        relative.strip_suffix(".html").unwrap_or(relative)
    }
}

fn live_targets(paths: &DigestPaths, base: &Url) -> Result<Vec<Target>> {
    let mut base = base.clone();
    ensure!(
        base.query().is_none() && base.fragment().is_none(),
        "URL base cannot contain a query or fragment"
    );
    let prefix = format!("{}/", base.path().trim_end_matches('/'));
    base.set_path(&prefix);
    discover(paths, false)?
        .into_iter()
        .filter(|rel| rel != Path::new("404.html"))
        .map(|rel| {
            let name = rel.to_str().context("invalid plaintext HTML path")?;
            let route = extensionless(name);
            let mut url = base.clone();
            // set_path encodes names without allowing them to replace the origin.
            url.set_path(&format!("{prefix}{route}"));
            Ok(Target {
                published: Published::Url(url),
                plaintext: paths.plain.join(rel),
                control: false,
            })
        })
        .collect()
}

fn local_targets(paths: &DigestPaths) -> Result<Vec<Target>> {
    Ok(discover(paths, true)?
        .into_iter()
        .map(|rel| Target {
            published: Published::File(paths.public.join(&rel)),
            plaintext: paths.plain.join(&rel),
            control: rel == Path::new("404.html"),
        })
        .collect())
}

pub fn verify_local(paths: &DigestPaths, encrypted: bool, password: Option<&str>) -> Result<usize> {
    let targets = local_targets(paths)?;
    let mut check = Compare::new(encrypted, password);
    for pair in &targets {
        let Published::File(published) = &pair.published else {
            unreachable!()
        };
        let actual = fs::read(published)
            .with_context(|| format!("read published page {}", published.display()))?;
        check
            .page(pair, &actual)
            .with_context(|| format!("verify {}", pair.published))?;
    }
    Ok(targets.len())
}

pub async fn verify_url_base(
    paths: &DigestPaths,
    base: &Url,
    encrypted: bool,
    password: Option<&str>,
) -> Result<usize> {
    verify_url_base_with_client(paths, base, encrypted, password, &SafeHttpClient::default()).await
}

pub async fn verify_url_base_with_client(
    paths: &DigestPaths,
    base: &Url,
    encrypted: bool,
    password: Option<&str>,
    client: &SafeHttpClient,
) -> Result<usize> {
    verify_targets(
        live_targets(paths, base)?,
        encrypted,
        password,
        false,
        client,
    )
    .await
}

pub async fn run(args: &VerifyArgs, paths: &DigestPaths, encrypted: bool) -> Result<usize> {
    if args.urls.len() != args.plains.len() {
        return Err(UnpairedUrls.into());
    }
    let mut pairs = Vec::new();
    if args.local {
        pairs.extend(local_targets(paths)?);
    }
    if let Some(base) = &args.url_base {
        pairs.extend(live_targets(paths, base)?);
    }
    pairs.extend(
        args.urls
            .iter()
            .cloned()
            .zip(&args.plains)
            .map(|(url, plain)| Target {
                published: Published::Url(url),
                plaintext: plain.clone(),
                control: false,
            }),
    );
    if pairs.is_empty() {
        return Err(NoTargets.into());
    }
    let password = if encrypted {
        Some(read_password(args.password_file.as_deref())?)
    } else {
        None
    };
    verify_targets(
        pairs,
        encrypted,
        password.as_deref(),
        true,
        &SafeHttpClient::default(),
    )
    .await
}

pub fn read_password(file: Option<&Path>) -> Result<String> {
    match file {
        Some(file) => crypto::read_password_file(file),
        None => {
            if std::env::var_os("DIGEST_SITE_PASSWORD_FILE").is_none() {
                if let Ok(password) = std::env::var("DIGEST_SITE_PASSWORD") {
                    ensure!(password.chars().count() >= 16, "site password is too short");
                    return Ok(password);
                }
            }
            crypto::read_password_file(&crypto::password_file_path()?)
        }
    }
}

async fn verify_targets(
    pairs: Vec<Target>,
    encrypted: bool,
    password: Option<&str>,
    verbose: bool,
    client: &SafeHttpClient,
) -> Result<usize> {
    let mut check = Compare::new(encrypted, password);
    let mut failed = 0;
    for pair in &pairs {
        let result: Result<usize> = async {
            let body = match &pair.published {
                Published::File(path) => fs::read(path)?,
                Published::Url(url) => {
                    let resp = client.fetch_exact_200_no_redirect(url).await?;
                    read_bytes_limited(resp, MAX_DECOMPRESSED_BYTES).await?
                }
            };
            check.page(pair, &body)
        }
        .await;
        match result {
            Ok(size) if verbose => println!("OK  {}  ({size} bytes, exact match)", pair.published),
            Ok(_) => {}
            Err(e) if verbose => {
                failed += 1;
                eprintln!("FAIL  {}: {e:#}", pair.published);
            }
            Err(e) => return Err(e).with_context(|| format!("verify {}", pair.published)),
        }
    }
    ensure!(failed == 0, "{failed} page(s) failed");
    if verbose {
        println!(
            "all {} page(s) {}matched plaintext",
            pairs.len(),
            if encrypted { "decrypted and " } else { "" }
        );
    }
    Ok(pairs.len())
}

struct Compare<'a> {
    encrypted: bool,
    password: Option<&'a str>,
    keys: HashMap<String, [u8; KEY_LEN]>,
}

impl<'a> Compare<'a> {
    fn new(encrypted: bool, password: Option<&'a str>) -> Self {
        Self {
            encrypted,
            password,
            keys: HashMap::new(),
        }
    }

    fn page(&mut self, target: &Target, body: &[u8]) -> Result<usize> {
        let expected = fs::read(&target.plaintext)
            .with_context(|| format!("read plaintext {}", target.plaintext.display()))?;
        let (pw, rm) = prompt_ids(std::str::from_utf8(body)?);
        let decrypted;
        let encrypted = self.encrypted && !target.control;
        let actual = if encrypted {
            ensure!(pw && rm, "body is missing the password prompt");
            let html = std::str::from_utf8(body)?;
            let envelope = crypto::extract_envelope(html)?;
            let password = self.password.context("verification password is missing")?;
            decrypted = if envelope.iter == PBKDF2_ITERATIONS {
                let key = match self.keys.get(&envelope.salt) {
                    Some(key) => *key,
                    None => {
                        let salt = STANDARD
                            .decode(&envelope.salt)
                            .context("invalid encryption salt")?;
                        ensure!(
                            salt.len() == crypto::SALT_LEN,
                            "invalid encryption salt length"
                        );
                        let key = crypto::derive_key(password, &salt);
                        self.keys.insert(envelope.salt.clone(), key);
                        key
                    }
                };
                crypto::decrypt_envelope_with_key(&envelope, &key)?
            } else {
                crypto::decrypt_envelope(&envelope, password)?
            };
            decrypted.as_slice()
        } else {
            ensure!(
                !pw && !rm,
                "public page unexpectedly contains a password prompt"
            );
            body
        };
        if actual != expected {
            let diff = actual
                .iter()
                .zip(&expected)
                .position(|(a, b)| a != b)
                .unwrap_or(actual.len().min(expected.len()));
            anyhow::bail!("{} content does not match plaintext (got {} bytes, plain {} bytes, first diff at {diff})",
                if encrypted { "decrypted" } else { "published" }, actual.len(), expected.len());
        }
        Ok(expected.len())
    }
}

fn prompt_ids(html: &str) -> (bool, bool) {
    let mut pw = false;
    let mut rm = false;
    for node in html_tokens(html) {
        if let Token::StartTag(tag) = node {
            if let Some(id) = tag.attributes.get(b"id".as_slice()) {
                pw |= id.as_slice() == b"pw";
                rm |= id.as_slice() == b"rm";
            }
        }
    }
    (pw, rm)
}
