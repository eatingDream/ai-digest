use std::{env, fmt::Write, fs, path::PathBuf};

const ASSETS: [(&str, &str, &str); 10] = [
    ("reader.css", "READER_CSS", "/* reader styles stub */"),
    ("index.css", "INDEX_CSS", "/* index styles stub */"),
    ("unlock.css", "UNLOCK_CSS", "/* unlock styles stub */"),
    ("404.css", "NOT_FOUND_CSS", "/* 404 styles stub */"),
    ("boot.js", "BOOT_JS", "/* boot stub */"),
    ("reader.js", "READER_JS", "/* reader stub */"),
    ("index.js", "INDEX_JS", "/* index stub */"),
    ("unlock.js", "UNLOCK_JS", "/* unlock stub */"),
    ("lock.js", "LOCK_JS", "/* lock stub */"),
    ("worker.js", "WORKER_JS", "const CFG = __WORKER_CFG_JSON__;\nexport default { async fetch() { return new Response(\"stub\"); } };\n"),
];
const FAVICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="7" fill="#b85432"/><path d="M8 24 14 8h4l6 16h-4l-1-4h-6l-1 4zm6-7h4l-2-6z" fill="#faf8f4"/></svg>"##;

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    println!("cargo:rerun-if-env-changed=AI_DIGEST_REQUIRE_WEB_DIST");
    println!("cargo:rerun-if-env-changed=AI_DIGEST_WEB_DIST");
    let required = env::var("AI_DIGEST_REQUIRE_WEB_DIST").as_deref() == Ok("1")
        || env::var("PROFILE").as_deref() == Ok("release");
    let mut generated = String::new();
    let web = env::var_os("AI_DIGEST_WEB_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("web/dist"));
    let mut using_stubs = false;
    for (name, ident, stub) in ASSETS {
        using_stubs |= !embed(&mut generated, &web.join(name), ident, stub, required);
    }
    writeln!(
        generated,
        "pub const USING_WEB_STUBS: bool = {using_stubs};"
    )
    .unwrap();
    embed(
        &mut generated,
        &root.join("docs/logo.svg"),
        "FAVICON_SVG",
        FAVICON,
        false,
    );
    embed_bytes(
        &mut generated,
        &root.join("docs/og-card.png"),
        "OG_CARD_PNG",
        OG_CARD_STUB,
    );
    fs::write(out.join("web_assets.rs"), generated).expect("write embedded asset manifest");
}

const OG_CARD_STUB: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn embed_bytes(generated: &mut String, path: &std::path::Path, ident: &str, stub: &[u8]) {
    println!("cargo:rerun-if-changed={}", path.display());
    if path
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
    {
        let abs = path.canonicalize().expect("canonicalize embedded bytes");
        writeln!(
            generated,
            "pub const {ident}: &[u8] = include_bytes!({abs:?});"
        )
        .unwrap();
    } else {
        writeln!(generated, "pub const {ident}: &[u8] = &{stub:?};").unwrap();
    }
}

fn embed(
    generated: &mut String,
    path: &std::path::Path,
    ident: &str,
    stub: &str,
    required: bool,
) -> bool {
    println!("cargo:rerun-if-changed={}", path.display());
    let source = path
        .metadata()
        .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
        .then(|| fs::read_to_string(path).expect("read embedded asset"))
        .filter(|text| !text.trim().is_empty());
    if let Some(source) = source {
        writeln!(generated, "pub const {ident}: &str = {source:?};").unwrap();
        true
    } else {
        assert!(
            !required,
            "missing or empty {}; run `npm --prefix web run build`",
            path.display(),
        );
        writeln!(generated, "pub const {ident}: &str = {stub:?};").unwrap();
        false
    }
}
