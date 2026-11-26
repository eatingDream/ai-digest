#!/usr/bin/env python3
"""Downstream security hardening, privacy scan, CSP hashing, and AES-256-GCM encryption for ai-digest."""
from __future__ import annotations

import base64
import gzip
import hashlib
import html as htmlmod
import json
import os
import re
import secrets
import stat
import sys
from pathlib import Path
from typing import Iterable

from cryptography.hazmat.primitives.ciphers.aead import AESGCM

import i18n

PASSWORD_FILE_DEFAULT = Path.home() / ".config/ai-digest/site-password"
SALT_FILE_DEFAULT = Path.home() / ".config/ai-digest/site-salt"
PASSWORD_ALPHABET = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789"  # gitleaks:allow (character set, not a secret)
PBKDF2_ITERATIONS = 600_000
SALT_LEN = 16
NONCE_LEN = 12
KEY_LEN = 32

PROMPT_MARK = 'id="rm"'  # the "remember this device" checkbox; language-neutral
STORE_KEY = "aie.k"
LOCK_MARK_START = "<!-- digest-lock:start -->"
LOCK_MARK_END = "<!-- digest-lock:end -->"
LOCK_JS = (
    "document.addEventListener('click',function(e){var b=e.target.closest&&e.target.closest('[data-lock]');"
    "if(!b)return;e.preventDefault();try{sessionStorage.removeItem('" + STORE_KEY + "');"
    "localStorage.removeItem('" + STORE_KEY + "')}catch(_){}location.reload()});"
)
LOCK_JS_BLOCK = f"{LOCK_MARK_START}\n<script>{LOCK_JS}</script>\n{LOCK_MARK_END}"

_FAVICON_HEADER = "/favicon.svg\n  Cache-Control: public, max-age=86400\n"
_SCRIPT_TAG_RE = re.compile(r"<script([^>]*)>(.*?)</script>", re.I | re.S)
_SCRIPT_NONEXEC_RE = re.compile(
    r"""\btype\s*=\s*["']?(?:application/(?:ld\+)?json|speculationrules)\b""", re.I
)
_SCRIPT_SRC_RE = re.compile(r"\bsrc\s*=", re.I)

PRIVACY_PATTERNS: list[tuple[str, str]] = [
    (r"[\w.+-]+@[\w.-]+\.\w+", "email"),
    (r"我(在)?(X|Twitter).{0,20}(关注|收藏|书签)", "x-circle"),
    (r"(X|Twitter).{0,8}圈子", "x-circle"),
    (r"我关注的人", "x-circle"),
    (r"我的(关注|收藏|书签)", "x-circle"),
    (r"\bI (follow|bookmarked|saved)\b.{0,30}\b(on )?(X|Twitter)\b", "x-circle"),
    (r"\b(people|accounts) I follow\b", "x-circle"),
    (r"\bmy (follows|bookmarks|likes|following list|X circle|Twitter circle)\b", "x-circle"),
    (r"\b(your|my) (X|Twitter) (circle|following)\b", "x-circle"),
    (r"(?:/Users/|/home/|C:\\Users\\)[A-Za-z0-9._-]+", "local-path"),
    (r"\b(?:ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AIza[0-9A-Za-z_-]{35}|AKIA[0-9A-Z]{16}|sk-[A-Za-z0-9_-]{24,})\b", "token-secret"),
    (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "private-key"),
]

_ROOT_CONTROL_HTML = frozenset({"404.html"})
_LANG_ZH_RE = re.compile(r'<div\s+class="lang-pane\s+lang-zh"[^>]*>', re.I)
_LANG_EN_RE = re.compile(r'<div\s+class="lang-pane\s+lang-en"[^>]*>', re.I)
_HTML_LANG_RE = re.compile(r'<html\b[^>]*\blang=["\']([^"\']+)["\']', re.I)
_LOCK_BTN_RE = re.compile(r'<button\s+type="button"\s+class="(?:ctrl-btn\s+)?lock"\s+data-lock\b[^>]*>.*?</button>\s*', re.I | re.S)


def _die(msg: str, code: int = 1) -> None:
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(code)


def _esc(s: str) -> str:
    return htmlmod.escape(s, quote=False).replace('"', "&quot;")


def _site_dir(base: Path, slug: str) -> Path:
    s = slug.strip("/")
    return (base / s) if s else base


def _strip_marked(html: str, start_mark: str, end_mark: str) -> str:
    pat = re.compile(re.escape(start_mark) + r".*?" + re.escape(end_mark) + r"\n?", flags=re.S)
    return pat.sub("", html)


def _iter_files(root: Path) -> list[Path]:
    if not root.exists():
        return []
    return sorted(p for p in root.rglob("*") if p.is_file() and p.name != ".DS_Store")


def _prune_empty_dirs(root: Path) -> None:
    if not root.is_dir():
        return
    for d in sorted((p for p in root.rglob("*") if p.is_dir()), key=lambda p: len(p.parts), reverse=True):
        try:
            d.rmdir()
        except OSError:
            pass


def iter_unencrypted_public(public_dir: Path) -> list[Path]:
    """Unencrypted root & public GEO files scanned by privacy_scan."""
    seen: set[Path] = set()
    out: list[Path] = []
    fixed = (
        "robots.txt",
        "_headers",
        "404.html",
        "favicon.svg",
        "_redirects",
        "sitemap.xml",
        "llms.txt",
        "llms-full.txt",
        "feed.xml",
        "zh/feed.xml",
        "en/feed.xml",
        "search-index.json",
        ".well-known/agent-skills/index.json",
        ".well-known/agent-skills/ai-digest-reader/SKILL.md",
    )
    for name in fixed:
        p = public_dir / name
        if p.is_file() and p not in seen:
            seen.add(p)
            out.append(p)
    if public_dir.is_dir():
        for p in sorted(public_dir.rglob("*.md")):
            if p.is_file() and p not in seen:
                seen.add(p)
                out.append(p)
    return out


def _render_security_headers(
    script_hashes: Iterable[str] = (), *, noindex: bool = True
) -> str:
    hashes = sorted(set(script_hashes))
    script_src = (
        " ".join(hashes) + " 'inline-speculation-rules'"
        if hashes
        else "'unsafe-inline'"
    )
    robots_line = "  X-Robots-Tag: noindex, nofollow\n" if noindex else ""
    return (
        "/*\n"
        f"{robots_line}"
        "  X-Content-Type-Options: nosniff\n"
        "  Referrer-Policy: no-referrer\n"
        "  X-Frame-Options: DENY\n"
        "  Strict-Transport-Security: max-age=31536000; includeSubDomains\n"
        "  Cross-Origin-Opener-Policy: same-origin\n"
        "  Cross-Origin-Resource-Policy: same-origin\n"
        "  Permissions-Policy: camera=(), microphone=(), geolocation=(), payment=(), usb=()\n"
        f"  Content-Security-Policy: default-src 'none'; script-src {script_src}; "
        "script-src-attr 'none'; style-src 'unsafe-inline'; img-src 'self'; font-src 'self'; "
        "connect-src 'self'; prefetch-src 'self'; frame-ancestors 'none'; base-uri 'none'; "
        "form-action 'none'; object-src 'none'; upgrade-insecure-requests\n"
        + _FAVICON_HEADER
    )


SECURITY_HEADERS = _render_security_headers()


def _collect_inline_script_hashes(paths: Iterable[Path]) -> list[str]:
    hashes: set[str] = set()
    for p in paths:
        if not p.is_file() or p.suffix.lower() != ".html":
            continue
        try:
            text = p.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for attrs, body in _SCRIPT_TAG_RE.findall(text):
            if _SCRIPT_SRC_RE.search(attrs) or _SCRIPT_NONEXEC_RE.search(attrs):
                continue
            norm = body.replace("\r\n", "\n").replace("\r", "\n")
            digest = base64.b64encode(hashlib.sha256(norm.encode("utf-8")).digest()).decode("ascii")
            hashes.add(f"'sha256-{digest}'")
    return sorted(hashes)


def finalize_csp_headers(
    public_dir: Path, plain_dir: Path, *, public_indexable: bool = False
) -> None:
    """Attach security headers and exact build-time SHA-256 script-src hashes into PUBLIC/_headers."""
    hp = public_dir / "_headers"
    if not hp.is_file():
        return
    htmls = [p for p in _iter_files(plain_dir) if p.suffix.lower() == ".html"]
    htmls.extend(p for p in _iter_files(public_dir) if p.suffix.lower() == ".html")
    hashes = _collect_inline_script_hashes(htmls)
    text = hp.read_text(encoding="utf-8")
    if "Content-Security-Policy:" in text:
        if not hashes:
            return
        script_src = " ".join(hashes) + " 'inline-speculation-rules'"
        updated = re.sub(
            r"(Content-Security-Policy:[^\n]*?script-src )[^;]+;",
            lambda m: m.group(1) + script_src + ";",
            text,
            count=1,
        )
    else:
        updated = _render_security_headers(hashes, noindex=not public_indexable) + text
    if updated != text:
        hp.write_text(updated, encoding="utf-8")


def _load_extra_privacy_patterns(site_dir: Path | None = None) -> list[tuple[str, str]]:
    env = os.environ.get("DIGEST_PRIVACY_PATTERNS_FILE", "").strip()
    if env:
        path = Path(env)
    elif site_dir is not None:
        path = site_dir / "privacy-patterns.txt"
    else:
        return []
    out: list[tuple[str, str]] = []
    if path.is_file():
        for line in path.read_text(encoding="utf-8").splitlines():
            line = line.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            pat, _, label = line.partition("\t")
            out.append((pat.strip(), label.strip() or "custom"))
    return out


def privacy_scan(paths: Iterable[Path], *, site_dir: Path | None = None) -> None:
    from html import unescape as _unescape

    hits: list[str] = []
    compiled: list[tuple[re.Pattern[str], str]] = []
    for pat, name in PRIVACY_PATTERNS + _load_extra_privacy_patterns(site_dir):
        try:
            compiled.append((re.compile(pat, re.I | re.S), name))
        except re.error as e:
            _die(f"invalid privacy pattern {pat!r} ({name}): {e}")
    for path in paths:
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        visible = _unescape(re.sub(r"<[^>]+>", "", text))
        views = (text, visible) if visible != text else (text,)
        seen_hits: set[tuple[str, str]] = set()
        for view in views:
            for rx, name in compiled:
                for m in rx.finditer(view):
                    snippet = m.group(0).replace("\n", " ")
                    if len(snippet) > 80:
                        snippet = snippet[:80] + "…"
                    key = (name, snippet)
                    if key not in seen_hits:
                        seen_hits.add(key)
                        hits.append(f"{path}: [{name}] {snippet}")
    if hits:
        print("privacy scan FAILED:", file=sys.stderr)
        for h in hits:
            print(f"  {h}", file=sys.stderr)
        _die("refusing to publish; remove personal info and retry")
    print("privacy scan: clean", file=sys.stderr)


def password_file_path() -> Path:
    env = os.environ.get("DIGEST_SITE_PASSWORD_FILE", "").strip()
    return Path(env) if env else PASSWORD_FILE_DEFAULT


def salt_file_path() -> Path:
    env = os.environ.get("DIGEST_SITE_SALT_FILE", "").strip()
    return Path(env) if env else SALT_FILE_DEFAULT


def _ensure_secret_parent(path: Path) -> None:
    d = path.parent
    d.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(d, 0o700)


def write_secret_file(path: Path, data: bytes) -> None:
    """Atomically write a mode-600 secret file. `data` must not be logged."""
    _ensure_secret_parent(path)
    tmp = path.with_name(path.name + ".tmp")
    fd = os.open(str(tmp), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(fd, "wb") as fh:
            fh.write(data)
            fh.flush()
        os.chmod(tmp, 0o600)
        os.replace(tmp, path)
        os.chmod(path, 0o600)
    except Exception:
        tmp.unlink(missing_ok=True)
        raise


def generate_and_save_password() -> None:
    """Write a new 20-char password to the password file. Never prints the value."""
    pw = "".join(secrets.choice(PASSWORD_ALPHABET) for _ in range(20))
    write_secret_file(password_file_path(), (pw + "\n").encode("utf-8"))
    pw = None  # noqa: F841
    path = password_file_path()
    print(f"wrote password to {path} (value not shown, mode 600)", file=sys.stderr)


def generate_and_save_salt() -> bytes:
    salt = secrets.token_bytes(SALT_LEN)
    write_secret_file(salt_file_path(), (salt.hex() + "\n").encode("ascii"))
    print(f"wrote new salt to {salt_file_path()} (remember-me keys invalidated)", file=sys.stderr)
    return salt


def rotate_password_and_salt() -> None:
    generate_and_save_password()
    generate_and_save_salt()


def read_password(password_override: str | None = None) -> str:
    if password_override is not None:
        return password_override
    path = password_file_path()
    if not path.is_file():
        _die(
            f"password file not found: {path}. "
            "Create it (20+ chars) or run --rotate-password."
        )
    mode = path.stat().st_mode
    if mode & (stat.S_IRGRP | stat.S_IROTH | stat.S_IWGRP | stat.S_IWOTH):
        print(f"warning: {path} should be mode 600", file=sys.stderr)
    pw = path.read_bytes().decode("utf-8")
    if pw.endswith("\n"):
        pw = pw[:-1]
    if pw.endswith("\r"):
        pw = pw[:-1]
    if len(pw) < 16:
        _die("password file is too short")
    return pw


def load_or_create_salt(salt_override: bytes | None = None) -> bytes:
    if salt_override is not None:
        return salt_override
    path = salt_file_path()
    if path.is_file():
        raw = path.read_text(encoding="ascii").strip()
        try:
            salt = bytes.fromhex(raw)
        except ValueError:
            _die(f"salt file {path} is not valid hex")
        if len(salt) < 16:
            _die("salt file is too short")
        return salt
    return generate_and_save_salt()


def derive_key(password: str, salt: bytes) -> bytes:
    return hashlib.pbkdf2_hmac(
        "sha256",
        password.encode("utf-8"),
        salt,
        PBKDF2_ITERATIONS,
        dklen=KEY_LEN,
    )


def localize_template(
    tmpl: str,
    lang: str | None = None,
    extras: dict[str, str] | None = None,
    *,
    default_lang: str = i18n.DEFAULT_LANG,
) -> str:
    """Fill {{key}} (HTML-escaped) and {{js:key}} (JSON string literal) from i18n.STRINGS."""
    lg = lang or default_lang
    extras = extras or {}

    def sub(m: re.Match) -> str:
        js, key = m.group(1) == "js:", m.group(2)
        if key in extras:
            return extras[key]
        if key in ("lang_switch", "demo_hint"):
            return ""
        try:
            val = i18n.t(lg, key)
        except KeyError:
            _die(f"password template uses unknown i18n key {key!r}")
        return json.dumps(val, ensure_ascii=False).replace("<", "\\u003c") if js else _esc(val)

    return re.sub(r"\{\{(js:)?(\w+)\}\}", sub, tmpl)


def _shell_lang_for_rel(rel: str, default_lang: str = i18n.DEFAULT_LANG) -> str:
    clean = rel.lstrip("/")
    if clean == "en/index.html" or clean.startswith("en/"):
        return "en"
    if clean == "zh/index.html" or clean.startswith("zh/"):
        return "zh-CN"
    return default_lang


def _lang_switch_href(rel: str = "", lang_alt: dict[str, str] | None = None) -> str | None:
    if not lang_alt:
        return None
    return lang_alt["base"] + rel.strip("/").removesuffix(".html").removesuffix("index")


def _lang_switch_link(
    rel: str = "",
    lang_alt: dict[str, str] | None = None,
    cls: str = "lang-switch",
) -> str:
    href = _lang_switch_href(rel, lang_alt)
    if href is None or not lang_alt:
        return ""
    code = lang_alt["code"]
    return (
        f'<a class="{cls}" href="{_esc(href)}" hreflang="{code}" lang="{code}">'
        f'{_esc(i18n.t(code, "lang_name"))}</a>'
    )


def shell_extras(
    rel: str = "",
    *,
    allow_lang_switch: bool = True,
    shell_lang: str | None = None,
    default_lang: str = i18n.DEFAULT_LANG,
    lang_alt: dict[str, str] | None = None,
    password_override: str | None = None,
) -> dict[str, str]:
    """Extra HTML for the password shell of the page at `rel`."""
    lg = shell_lang or default_lang
    extras = {"lang_switch": _lang_switch_link(rel, lang_alt, "lang-alt") if allow_lang_switch else ""}
    if password_override is not None:  # demo builds only: the demo password is public by design
        extras["demo_hint"] = (
            '<p class="demo-hint">'
            + _esc(i18n.t(lg, "pw_demo_hint")).replace("{pw}", f"<code>{_esc(password_override)}</code>")
            + "</p>"
        )
    return extras


def wrap_encrypted(
    plaintext: str,
    key: bytes,
    salt: bytes,
    tmpl: str,
    rel: str = "",
    *,
    allow_lang_switch: bool = True,
    shell_lang: str | None = None,
    default_lang: str = i18n.DEFAULT_LANG,
    lang_alt: dict[str, str] | None = None,
    password_override: str | None = None,
) -> str:
    nonce = os.urandom(NONCE_LEN)
    packed = gzip.compress(plaintext.encode("utf-8"), compresslevel=9, mtime=0)
    ct = AESGCM(key).encrypt(nonce, packed, None)
    cfg = {
        "v": 2,
        "z": "gzip",
        "iter": PBKDF2_ITERATIONS,
        "salt": base64.b64encode(salt).decode("ascii"),
        "iv": base64.b64encode(nonce).decode("ascii"),
        "ct": base64.b64encode(ct).decode("ascii"),
    }
    if "__ENCRYPT_CFG_JSON__" not in tmpl:
        _die("password template is missing the __ENCRYPT_CFG_JSON__ placeholder")
    lg = shell_lang or _shell_lang_for_rel(rel, default_lang)
    tmpl = localize_template(
        tmpl,
        lang=lg,
        extras=shell_extras(
            rel,
            allow_lang_switch=allow_lang_switch,
            shell_lang=lg,
            default_lang=default_lang,
            lang_alt=lang_alt,
            password_override=password_override,
        ),
        default_lang=default_lang,
    )
    return tmpl.replace(
        "__ENCRYPT_CFG_JSON__",
        json.dumps(cfg, separators=(",", ":")),
        1,
    )


def _distinctive_snippets(plain: str, tmpl: str) -> list[str]:
    snippets: list[str] = []
    for m in re.finditer(r"<h[1-6][^>]*>(.*?)</h[1-6]>", plain, re.I | re.S):
        t = re.sub(r"<[^>]+>", "", m.group(1))
        t = re.sub(r"\s+", " ", t).strip()
        if len(t) >= 8 and t not in tmpl:
            snippets.append(t)
    seen: set[str] = set()
    out: list[str] = []
    for s in snippets:
        if s not in seen:
            seen.add(s)
            out.append(s)
    return out[:30]


def assert_no_plaintext_leak(
    plain: str,
    wrapped: str,
    rel: Path,
    tmpl: str,
    *,
    allow_lang_switch: bool = True,
    shell_lang: str | None = None,
    default_lang: str = i18n.DEFAULT_LANG,
    lang_alt: dict[str, str] | None = None,
    password_override: str | None = None,
) -> None:
    lg = shell_lang or _shell_lang_for_rel(rel.as_posix(), default_lang)
    tmpl = localize_template(
        tmpl,
        lang=lg,
        extras=shell_extras(
            rel.as_posix(),
            allow_lang_switch=allow_lang_switch,
            shell_lang=lg,
            default_lang=default_lang,
            lang_alt=lang_alt,
            password_override=password_override,
        ),
        default_lang=default_lang,
    )
    for snip in _distinctive_snippets(plain, tmpl):
        if snip in wrapped:
            _die(f"plaintext leak in encrypted {rel}: {snip!r}")
    for needle in (i18n.t(lg, "pw_title"), i18n.t(lg, "pw_label"), i18n.t(lg, "pw_open"), PROMPT_MARK, "noindex"):
        if needle not in wrapped:
            _die(f"encrypted {rel} is missing the password prompt ({needle})")
    if "__ENCRYPT_CFG_JSON__" in wrapped:
        _die(f"encrypted {rel} still has an unreplaced template placeholder")
    if '"ct"' not in wrapped and '"ct":' not in wrapped:
        _die(f"encrypted {rel} is missing ciphertext")


def _inject_lock_btn_in_fragment(fragment: str, lang: str) -> str:
    lock_btn = (
        f'<button type="button" class="lock" data-lock '
        f'title="{_esc(i18n.t(lang, "lock_title"))}">{_esc(i18n.t(lang, "lock"))}</button>'
    )
    return re.sub(
        r'(<nav\s+class="issue-nav\s+top"[^>]*>.*?)(</nav>)',
        lambda m: m.group(1) + lock_btn + m.group(2),
        fragment,
        count=1,
        flags=re.S | re.I,
    )


def inject_lock_ui_html(html: str, default_lang: str = i18n.DEFAULT_LANG) -> str:
    """Idempotently inject downstream lock button(s) and LOCK_JS_BLOCK into plaintext HTML."""
    html = _strip_marked(html, LOCK_MARK_START, LOCK_MARK_END)
    html = _LOCK_BTN_RE.sub("", html)

    m_lang = _HTML_LANG_RE.search(html)
    doc_lang = default_lang
    if m_lang:
        try:
            doc_lang = i18n.normalize_lang(m_lang.group(1))
        except ValueError:
            doc_lang = default_lang

    zh_m = _LANG_ZH_RE.search(html)
    en_m = _LANG_EN_RE.search(html)
    if zh_m and en_m:
        first_m, second_m = (zh_m, en_m) if zh_m.start() < en_m.start() else (en_m, zh_m)
        first_lang = "zh-CN" if first_m is zh_m else "en"
        second_lang = "en" if first_m is zh_m else "zh-CN"
        part1 = _inject_lock_btn_in_fragment(html[: second_m.start()], first_lang)
        part2 = _inject_lock_btn_in_fragment(html[second_m.start():], second_lang)
        html = part1 + part2
    elif '<nav class="issue-nav top"' in html:
        html = _inject_lock_btn_in_fragment(html, doc_lang)
    elif '<button type="button" class="ctrl-btn theme-btn' in html:
        idx_btn = (
            f'<button type="button" class="ctrl-btn lock" data-lock '
            f'title="{_esc(i18n.t(doc_lang, "lock_title"))}">{_esc(i18n.t(doc_lang, "lock"))}</button>'
        )
        html = html.replace('<button type="button" class="ctrl-btn theme-btn', idx_btn + '<button type="button" class="ctrl-btn theme-btn', 1)

    if "</body>" in html.lower():
        html = re.sub(r"</body>", lambda m: LOCK_JS_BLOCK + "\n" + m.group(0), html, count=1, flags=re.I)
    return html


def _load_enc_cache(enc_cache_path: Path) -> dict[str, str]:
    try:
        return json.loads(enc_cache_path.read_text(encoding="utf-8"))
    except Exception:
        return {}


def _save_enc_cache(enc_cache_path: Path, cache: dict[str, str]) -> None:
    enc_cache_path.parent.mkdir(parents=True, exist_ok=True)
    tmp = enc_cache_path.with_name(enc_cache_path.name + ".tmp")
    tmp.write_text(json.dumps(cache, indent=1, sort_keys=True), encoding="utf-8")
    os.replace(tmp, enc_cache_path)


def encrypt_all_pages(
    cfg: dict[str, str],
    *,
    plain_dir: Path,
    public_dir: Path,
    template_path: Path,
    enc_cache_path: Path,
    default_lang: str = i18n.DEFAULT_LANG,
    lang_alt: dict[str, str] | None = None,
    demo_mode: bool = False,
    password_override: str | None = None,
    salt_override: bytes | None = None,
    root_latest_rels: Iterable[str] = (),
) -> list[str]:
    """Inject lock UI downstream and encrypt every HTML page under plain_dir into public_dir (ENCRYPT=1)."""
    if not template_path.is_file():
        _die(f"password template not found: {template_path}")
    tmpl = template_path.read_text(encoding="utf-8")
    password = read_password(password_override)
    salt = load_or_create_salt(salt_override)
    key = derive_key(password, salt)
    password = None  # noqa: F841

    slug = cfg["SLUG"]
    src_root = _site_dir(plain_dir, slug)
    dst_root = _site_dir(public_dir, slug)
    if not src_root.is_dir():
        _die("plaintext pages not found under site/plain; run a rebuild first")
    html_files = sorted(src_root.rglob("*.html"))
    if not html_files:
        _die("no plaintext HTML to encrypt")

    cache = _load_enc_cache(enc_cache_path)
    salt_fp = hashlib.sha256(salt).hexdigest()[:16]
    key_fp = hashlib.sha256(key).hexdigest()[:16]
    tmpl_fp = hashlib.sha256(tmpl.encode("utf-8")).hexdigest()[:16]
    new_cache: dict[str, str] = {}
    n = skipped = 0
    changed_pages: list[str] = []

    def _encrypt_one(
        src: Path,
        dest: Path,
        rel: Path,
        cache_key: str,
        change_name: str,
        *,
        allow_lang_switch: bool = True,
    ) -> None:
        nonlocal n, skipped
        raw_plain = src.read_text(encoding="utf-8")
        shell_lg = _shell_lang_for_rel(rel.as_posix(), default_lang)
        plain = inject_lock_ui_html(raw_plain, shell_lg)
        if plain != raw_plain:
            src.write_text(plain, encoding="utf-8")
        alt_href = _lang_switch_href(rel.as_posix(), lang_alt) if allow_lang_switch else ""
        alt_fp = hashlib.sha256((alt_href or "").encode()).hexdigest()[:8]
        fp = f"{hashlib.sha256(plain.encode('utf-8')).hexdigest()}:{salt_fp}:{key_fp}:{tmpl_fp}:{alt_fp}:{shell_lg}"
        if cache.get(cache_key) == fp and dest.is_file():
            body = dest.read_text(encoding="utf-8")
            if PROMPT_MARK in body and '"ct":' in body:
                new_cache[cache_key] = fp
                skipped += 1
                return
        wrapped = wrap_encrypted(
            plain,
            key,
            salt,
            tmpl,
            rel.as_posix(),
            allow_lang_switch=allow_lang_switch,
            shell_lang=shell_lg,
            default_lang=default_lang,
            lang_alt=lang_alt,
            password_override=password_override,
        )
        assert_no_plaintext_leak(
            plain,
            wrapped,
            rel,
            tmpl,
            allow_lang_switch=allow_lang_switch,
            shell_lang=shell_lg,
            default_lang=default_lang,
            lang_alt=lang_alt,
            password_override=password_override,
        )
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(wrapped, encoding="utf-8")
        new_cache[cache_key] = fp
        changed_pages.append(change_name)
        n += 1

    for src in html_files:
        rel = src.relative_to(src_root)
        key_rel = rel.as_posix()
        _encrypt_one(src, dst_root / rel, rel, key_rel, key_rel, allow_lang_switch=True)

    keep = {s.relative_to(src_root).as_posix() for s in html_files}
    if dst_root.is_dir():
        for pub in sorted(dst_root.rglob("*.html")):
            r = pub.relative_to(dst_root).as_posix()
            if not slug and r in _ROOT_CONTROL_HTML:
                continue
            if r not in keep:
                pub.unlink()
                changed_pages.append(f"-{r}")

    if slug and not demo_mode:
        for rel_str in root_latest_rels:
            if rel_str.startswith(f"{slug}/"):
                continue
            src = plain_dir / rel_str
            dest = public_dir / rel_str
            if src.is_file():
                _encrypt_one(
                    src,
                    dest,
                    Path(rel_str),
                    f"__root__/{rel_str}",
                    f"/{rel_str}",
                    allow_lang_switch=False,
                )
            elif dest.is_file():
                dest.unlink()
                changed_pages.append(f"-/{rel_str}")

    _prune_empty_dirs(public_dir)
    _save_enc_cache(enc_cache_path, new_cache)
    print(
        f"encrypted {n} HTML page(s), reused {skipped} unchanged, with AES-256-GCM / "
        f"PBKDF2-SHA256 ({PBKDF2_ITERATIONS} iterations)",
        file=sys.stderr,
    )
    return changed_pages


def _check_unencrypted_root_files(public_dir: Path) -> None:
    for name in ("robots.txt", "_headers", "404.html"):
        path = public_dir / name
        if not path.is_file():
            _die(f"missing unencrypted root file: {name}")
        body = path.read_text(encoding="utf-8")
        if PROMPT_MARK in body or "__ENCRYPT_CFG_JSON__" in body:
            _die(f"{name} must stay unencrypted")
    redirects_path = public_dir / "_redirects"
    if redirects_path.is_file():
        body = redirects_path.read_text(encoding="utf-8")
        if PROMPT_MARK in body or "__ENCRYPT_CFG_JSON__" in body:
            _die("_redirects must stay unencrypted")


def assert_public_matches_plain(
    cfg: dict[str, str],
    *,
    plain_dir: Path,
    public_dir: Path,
    demo_mode: bool = False,
    root_latest_rels: Iterable[str] = (),
) -> None:
    """Verify public HTML pages match site/plain/ verbatim with no password prompt (ENCRYPT=0)."""
    slug = cfg["SLUG"]
    src_root = _site_dir(plain_dir, slug)
    dst_root = _site_dir(public_dir, slug)
    pages = (
        [
            p
            for p in sorted(dst_root.rglob("*.html"))
            if slug or p.relative_to(dst_root).as_posix() not in _ROOT_CONTROL_HTML
        ]
        if dst_root.exists()
        else []
    )
    if not pages:
        _die("no published HTML under public/; run a rebuild first")
    checked = 0

    def _check_plain(path: Path, plain_path: Path) -> None:
        nonlocal checked
        text = path.read_text(encoding="utf-8")
        if PROMPT_MARK in text or "__ENCRYPT_CFG_JSON__" in text:
            _die(f"{path.relative_to(public_dir)} contains password prompt despite ENCRYPT=0; run --rebuild")
        if not plain_path.is_file():
            _die(f"missing plaintext source for {path.relative_to(public_dir)}; run --rebuild")
        if text != plain_path.read_text(encoding="utf-8"):
            _die(f"{path.relative_to(public_dir)} does not match plaintext; run --rebuild")
        checked += 1

    for path in pages:
        rel = path.relative_to(dst_root)
        _check_plain(path, src_root / rel)
    if slug and not demo_mode:
        for rel_str in root_latest_rels:
            if rel_str.startswith(f"{slug}/"):
                continue
            path = public_dir / rel_str
            if path.is_file():
                _check_plain(path, plain_dir / rel_str)
    _check_unencrypted_root_files(public_dir)
    print(f"public check: {checked} page(s) match plaintext (ENCRYPT=0)", file=sys.stderr)


def assert_public_is_encrypted(
    cfg: dict[str, str],
    *,
    plain_dir: Path,
    public_dir: Path,
    template_path: Path,
    default_lang: str = i18n.DEFAULT_LANG,
    lang_alt: dict[str, str] | None = None,
    demo_mode: bool = False,
    password_override: str | None = None,
    root_latest_rels: Iterable[str] = (),
) -> None:
    """Refuse to deploy if issue/index/latest HTML is still plaintext."""
    slug = cfg["SLUG"]
    dst_root = _site_dir(public_dir, slug)
    src_root = _site_dir(plain_dir, slug)
    pages = (
        [
            p
            for p in sorted(dst_root.rglob("*.html"))
            if slug or p.relative_to(dst_root).as_posix() not in _ROOT_CONTROL_HTML
        ]
        if dst_root.exists()
        else []
    )
    if not pages:
        _die("no published HTML under public/; run a rebuild first")
    tmpl = template_path.read_text(encoding="utf-8") if template_path.is_file() else ""
    checked = 0

    def _check_page(path: Path, rel: Path, plain_path: Path, *, allow_lang_switch: bool) -> None:
        nonlocal checked
        text = path.read_text(encoding="utf-8")
        lg = _shell_lang_for_rel(rel.as_posix(), default_lang)
        if PROMPT_MARK not in text or i18n.t(lg, "pw_title") not in text:
            _die(f"{path.relative_to(public_dir)} does not look encrypted; run --rebuild")
        if "__ENCRYPT_CFG_JSON__" in text:
            _die(f"{path.relative_to(public_dir)} has an unreplaced placeholder")
        if plain_path.is_file():
            assert_no_plaintext_leak(
                plain_path.read_text(encoding="utf-8"),
                text,
                rel,
                tmpl,
                allow_lang_switch=allow_lang_switch,
                shell_lang=lg,
                default_lang=default_lang,
                lang_alt=lang_alt,
                password_override=password_override,
            )
        checked += 1

    for path in pages:
        rel = path.relative_to(dst_root)
        _check_page(path, rel, src_root / rel, allow_lang_switch=True)
    if slug and not demo_mode:
        for rel_str in root_latest_rels:
            if rel_str.startswith(f"{slug}/"):
                continue
            path = public_dir / rel_str
            if path.is_file():
                _check_page(path, Path(rel_str), plain_dir / rel_str, allow_lang_switch=False)
    _check_unencrypted_root_files(public_dir)
    print(f"encryption check: {checked} page(s) look encrypted", file=sys.stderr)
