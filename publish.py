#!/usr/bin/env python3
"""Publish an AI digest HTML issue to the public Cloudflare Pages site.

Usage (routine runs -- incremental, quiet, prints ~6 lines):
  python3 publish.py --issue daily/2026-11-16 --md digest-2026-11-16.md [--md-en digest-2026-11-16.en.md]
  python3 publish.py --issue weekly/2026-11-16_11-22 --md weekly-2026-11-16_11-22.md
  python3 publish.py --remove weekly/2026-11-09_11-13
  (renders that one issue — bundling a sibling *.en.md automatically when LANG=zh-CN —
  rebuilds nav/index from site/sources, re-encrypts only pages whose plaintext changed
  (site/enc-cache.json), deploys, checks HTTP + decrypt; full log in logs/publish-*.log)

Legacy usage:
  python3 publish.py <html_path> <daily|weekly> <date_label>
  python3 publish.py --rebuild
  python3 publish.py --rerender-all   (re-render all issues from md after a template change, then deploy)
  python3 publish.py --deploy-only
  python3 publish.py --prepare-only <html_path> <daily|weekly> <date_label>
  python3 publish.py --rotate-password

Examples:
  python3 publish.py html/daily-2026-11-13.html daily 2026-11-13
  python3 publish.py html/weekly-2026-11-09_11-13.html weekly 2026-11-09_11-13

What it does:
  1. Loads site/CONFIG (or env SITE_DOMAIN / SITE_SLUG / CF_ACCOUNT_ID / CF_PAGES_PROJECT /
     CF_ZONE_ID; env wins), creating it with a random unguessable slug if absent.
  2. Saves a clean source copy under site/sources/ (no nav).
  3. Injects noindex meta if missing, injects Back to index / Previous / Next / This week's recap
     nav (labels follow the site language, DIGEST_LANG) (same-type neighbors; daily also links the weekly that covers that day).
     Nav is injected at publish time, not by render_html.py. Every publish
     regenerates nav on ALL issues so neighbors stay consistent.
  4. Regenerates the index at /<slug>/ (daily and weekly issues, newest first).
     Plaintext after nav lives in site/plain/ (gitignored, never deployed).
  5. Runs a privacy scan on the plaintext (and on unencrypted root files);
     fails on hits. The scan runs BEFORE encryption, never on ciphertext.
  6. Encrypts every published HTML page (each issue AND the index) with
     AES-256-GCM, key from PBKDF2-SHA256 (600_000 iterations) and a shared
     random salt. robots.txt, 404.html, _headers, and _redirects stay unencrypted.
     The password is read from the file named by env DIGEST_SITE_PASSWORD_FILE
     (default ~/.config/ai-digest/site-password) and is never printed or written
     into CONFIG, this script, git, or the site tree.
  7. Deploys the static public/ tree to Cloudflare Pages (no Functions, no
     Worker script) and prints the public URLs.

  --rotate-password generates a new password (same file, mode 600) and a
  new salt, re-encrypts every issue from site/sources, and redeploys
  (unless --prepare-only). It never prints the password.

Auth (for unattended deploys):
  The script never stores a Cloudflare credential in CONFIG, in the site tree,
  or in this file. It looks for a token in this order:
    1. Environment variable CLOUDFLARE_API_TOKEN
    2. Environment variable CLOUDFLARE_API_TOKEN_FILE (path to a file)
    3. File ~/.config/ai-digest/cloudflare_token  (mode 600 recommended)

  Create a scoped Account API token in the Cloudflare dashboard:
    https://dash.cloudflare.com/profile/api-tokens
    Permissions (minimum):
      - Account · Cloudflare Pages · Edit   (enough for later publishes)
      - Zone · DNS · Edit, limited to your zone
        (only needed the first time, to bind SITE_DOMAIN)
    Account Resources: include your Cloudflare account (CF_ACCOUNT_ID).

  Then:
    umask 077
    install -m 600 /dev/stdin ~/.config/ai-digest/cloudflare_token
    # paste token, Ctrl-D; do not put the token in chat or in this repo.

  Pages Direct Upload asset APIs need a scoped API token (they reject
  tokens/JWTs issued through some MCP sessions). With a token, this script calls
  Wrangler (`npx wrangler pages
  deploy`) so hashing and the JWT upload follow Cloudflare's official path.
  There is no Functions directory and no Worker script: static asset
  requests stay free and unlimited on the free plan.

  The first-time Pages project create and (if the token also has Zone DNS
  Edit on your zone) the SITE_DOMAIN binding are done by --deploy-only
  / a full publish. If DNS Edit is missing, bind the domain afterwards
  in the dashboard or via the API:
    POST /accounts/<CF_ACCOUNT_ID>/pages/projects/<CF_PAGES_PROJECT>/domains
    body {"name": "<SITE_DOMAIN>"}

Demo / CI (no deploy, no Cloudflare, no secrets on disk):
  DIGEST_DEMO_PASSWORD=demo python3 publish.py --demo --out dist/               # English /en/ + Chinese /zh-CN/
  DIGEST_DEMO_PASSWORD=demo python3 publish.py --demo --lang en --out dist/     # one language, at /demo/
  builds examples/*.md into an encrypted static site under dist/ (see README). With both
  languages every page links to its twin in the other language and `/` redirects to /en/.

Exit codes: 0 ok; 1 any failure (message on stderr, prefixed "error:"); 2 bad command line.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import html as htmlmod
import json
import mimetypes
import os
import re
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from datetime import date, datetime, timedelta
from pathlib import Path
from typing import Iterable
from zoneinfo import ZoneInfo

import i18n
from render_html import FAVICON_SVG, _TAG_SPLIT_RE
import site_crypto
from site_crypto import (
    KEY_LEN,
    LOCK_JS,
    LOCK_JS_BLOCK,
    LOCK_MARK_END,
    LOCK_MARK_START,
    NONCE_LEN,
    PASSWORD_ALPHABET,
    PASSWORD_FILE_DEFAULT,
    PBKDF2_ITERATIONS,
    PRIVACY_PATTERNS,
    PROMPT_MARK,
    SALT_FILE_DEFAULT,
    SALT_LEN,
    SECURITY_HEADERS,
    STORE_KEY,
    _SCRIPT_NONEXEC_RE,
    _SCRIPT_SRC_RE,
    _SCRIPT_TAG_RE,
    _collect_inline_script_hashes,
    _render_security_headers,
    derive_key,
    generate_and_save_password,
    generate_and_save_salt,
    password_file_path,
    rotate_password_and_salt,
    salt_file_path,
    write_secret_file,
)
from version import __version__

CODE_ROOT = Path(__file__).resolve().parent
ROOT = Path(os.environ.get("DIGEST_ROOT") or CODE_ROOT)
SITE = ROOT / "site"
PUBLIC = SITE / "public"
PLAIN = SITE / "plain"
SOURCES = SITE / "sources"
CONFIG_PATH = SITE / "CONFIG"
TEMPLATE_PATH = (SITE / "password_template.html") if (SITE / "password_template.html").is_file() else (CODE_ROOT / "site" / "password_template.html")
TOKEN_FILE_DEFAULT = Path.home() / ".config/ai-digest/cloudflare_token"

# Defaults when optional keys are unset in site/CONFIG or env (see load_config).
ACCOUNT_ID_DEFAULT = ""
ZONE_ID_DEFAULT = ""
PROJECT_DEFAULT = "ai-digest"
DOMAIN_DEFAULT = "digest.example.com"
PLATFORM = "pages"
CF_API = "https://api.cloudflare.com/client/v4"
WRANGLER_VERSION = os.environ.get("WRANGLER_VERSION", "4.166.0").strip() or "4.166.0"
FAVICON_LINK = '<link rel="icon" type="image/svg+xml" href="/favicon.svg">'


def _finalize_csp_headers(ctx: "SiteContext | None" = None) -> None:
    """Replace script-src 'unsafe-inline' in PUBLIC/_headers with exact build-time SHA-256 hashes."""
    c = ctx or _default_ctx()
    site_crypto.finalize_csp_headers(c.public, c.plain)


# Output language of the site (en | zh-CN); resolved by load_config() / --lang, see i18n.py.
LANG = i18n.DEFAULT_LANG
CLI_LANG: str | None = None
_DEMO_MODE: bool = False
# Companion site in the other language, e.g. {"code": "zh-CN", "base": "/zh-CN/"}.
# Set via LANG_ALT in site/CONFIG (or DIGEST_LANG_ALT env), or automatically in --demo.
LANG_ALT: dict[str, str] | None = None


@dataclass
class SiteContext:
    """Explicit isolated build/publish paths and flags (replaces module globals for demo & callers)."""

    root: Path
    site: Path
    public: Path
    plain: Path
    sources: Path
    config_path: Path
    template_path: Path
    enc_cache_path: Path
    lang: str = i18n.DEFAULT_LANG
    cli_lang: str | None = None
    lang_alt: dict[str, str] | None = None
    demo_mode: bool = False
    password_override: str | None = None
    salt_override: bytes | None = None
    changed_pages: list[str] = field(default_factory=list)
    env_overrides: dict[str, str | None] | None = None

    def env_get(self, key: str, default: str = "") -> str:
        if self.env_overrides is not None and key in self.env_overrides:
            val = self.env_overrides[key]
            return "" if val is None else val
        return os.environ.get(key, default)

    def env_has(self, key: str) -> bool:
        if self.env_overrides is not None and key in self.env_overrides:
            return self.env_overrides[key] is not None
        return key in os.environ


def _default_ctx() -> SiteContext:
    return SiteContext(
        root=ROOT,
        site=SITE,
        public=PUBLIC,
        plain=PLAIN,
        sources=SOURCES,
        config_path=CONFIG_PATH,
        template_path=TEMPLATE_PATH,
        enc_cache_path=ENC_CACHE_PATH,
        lang=LANG,
        cli_lang=CLI_LANG,
        lang_alt=LANG_ALT,
        demo_mode=_DEMO_MODE,
        password_override=_PASSWORD_OVERRIDE,
        salt_override=_SALT_OVERRIDE,
        changed_pages=CHANGED_PAGES,
    )

# Markers used so republishing can replace nav without touching renderer output.
CSS_MARK_START = "<!-- digest-nav-css:start -->"
CSS_MARK_END = "<!-- digest-nav-css:end -->"
META_MARK_START = "<!-- digest-meta:start -->"
META_MARK_END = "<!-- digest-meta:end -->"
NAV_TOP_START = "<!-- digest-nav:top:start -->"
NAV_TOP_END = "<!-- digest-nav:top:end -->"
NAV_BOT_START = "<!-- digest-nav:bottom:start -->"
NAV_BOT_END = "<!-- digest-nav:bottom:end -->"
PERF_MARK_START = "<!-- digest-perf:start -->"
PERF_MARK_END = "<!-- digest-perf:end -->"
LOCK_MARK_START = "<!-- digest-lock:start -->"
LOCK_MARK_END = "<!-- digest-lock:end -->"
LANG_BTN_START = "<!-- digest-lang-btn:start -->"
LANG_BTN_END = "<!-- digest-lang-btn:end -->"

NAV_CSS = """
.issue-nav {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 14px;
  padding: 6px 0 8px;
  font-size: 12.5px;
  letter-spacing: 0.03em;
  color: var(--ink-4);
}
.issue-nav.top { border-top: 1px solid var(--line-soft); padding: 4px 0 7px; font-size: 12px; }
.issue-nav.bottom {
  border-top: 1px solid var(--line);
  margin-top: 28px;
  padding: 18px 0 0;
}
.issue-nav a, .issue-nav .lock {
  display: inline-flex;
  align-items: center;
  padding: 2px 7px;
  margin: 0 -4px;
  border-radius: 999px;
  transition: transform 240ms var(--spring, cubic-bezier(0.34, 1.56, 0.64, 1)),
              background-color 140ms ease, color 140ms ease;
}
.issue-nav a { color: var(--accent-ink); text-decoration: none; }
.issue-nav a:hover {
  background: var(--accent-softer, rgba(184, 84, 50, 0.06));
  transform: translateY(-1px);
}
.issue-nav a:active, .issue-nav .lock:active { transform: scale(0.94); transition-duration: 80ms; }
.issue-nav .gone { color: var(--ink-4); opacity: 0.45; }
.issue-nav .sep {
  width: 1px;
  height: 11px;
  background: var(--line-strong);
  display: inline-block;
}
.issue-nav .week-link { margin-left: auto; }
.issue-nav .lock {
  border: 0; background: none; font: inherit; letter-spacing: inherit;
  color: var(--ink-4); cursor: pointer;
}
.issue-nav .lock:hover { color: var(--accent-ink); background: var(--well, rgba(128,128,128,.08)); }
.issue-nav .lang-switch { color: var(--ink-4); }
@media (max-width: 560px) {
  .issue-nav { gap: 4px 10px; font-size: 12px; }
  .issue-nav .week-link { margin-left: 0; }
}
""".strip()


def perf_head_html(hints: list[str], slug: str | None = None, *, ctx: SiteContext | None = None) -> str:
    """Prefetch hints + speculation rules (prerender same-site links on intent)."""
    raw_slug = load_config(ctx)["SLUG"] if slug is None else slug
    s = raw_slug.strip("/")
    match_pat = f"/{s}/*" if s else "/*"
    seen: list[str] = []
    for h in hints:
        if h and h not in seen:
            seen.append(h)
    links = "".join(f'<link rel="prefetch" href="{_esc(h)}">\n' for h in seen)
    rules = json.dumps(
        {
            "prerender": [
                {
                    "where": {"and": [
                        {"href_matches": match_pat},
                        {"not": {"selector_matches": "[data-lock], [target=_blank]"}},
                    ]},
                    "eagerness": "moderate",
                }
            ]
        },
        separators=(",", ":"),
    ).replace("<", "\\u003c")
    return (
        f"{PERF_MARK_START}\n{links}"
        f'<script type="speculationrules">{rules}</script>\n{PERF_MARK_END}\n'
    )


def minify_css(html: str) -> str:
    """Conservative whitespace/comment minifier for inline <style> blocks."""
    def _min(m: re.Match) -> str:
        css = m.group(2)
        css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
        css = re.sub(r"\s+", " ", css)
        css = re.sub(r"\s*([{};,>])\s*", r"\1", css)
        css = css.replace(";}", "}")
        return f"{m.group(1)}{css.strip()}</style>"
    return re.sub(r"(<style[^>]*>)(.*?)</style>", _min, html, flags=re.S | re.I)


def _site_dir(base: Path, slug: str) -> Path:
    s = slug.strip("/")
    return (base / s) if s else base


def _url_prefix(slug: str, sub: str = "") -> str:
    parts = [p for p in (slug.strip("/"), sub.strip("/")) if p]
    return ("/" + "/".join(parts)) if parts else ""


def issue_prefetch_hints(
    issue: "Issue",
    issues: list["Issue"],
    *,
    slug: str | None = None,
    abs_links: bool = False,
    url_prefix: str | None = None,
) -> list[str]:
    same = sorted([i for i in issues if i.kind == issue.kind], key=lambda x: (x.start, x.end, x.label))
    idx = next(n for n, i in enumerate(same) if i.label == issue.label)
    if abs_links:
        s = load_config()["SLUG"] if slug is None else slug
        prefix = url_prefix if url_prefix is not None else _url_prefix(s)
        hints = [f"{prefix}/"]
        if idx + 1 < len(same):
            hints.append(f"{prefix}/{issue.kind}/{same[idx + 1].label}")
        if idx > 0:
            hints.append(f"{prefix}/{issue.kind}/{same[idx - 1].label}")
        if issue.kind == "daily":
            ws = sorted([w for w in issues if w.kind == "weekly" and w.start <= issue.start <= w.end],
                        key=lambda w: (w.end, w.start), reverse=True)
            if ws:
                hints.append(f"{prefix}/weekly/{ws[0].label}")
        return hints
    hints = ["../"]
    if idx + 1 < len(same):
        hints.append(same[idx + 1].label)
    if idx > 0:
        hints.append(same[idx - 1].label)
    if issue.kind == "daily":
        ws = sorted([w for w in issues if w.kind == "weekly" and w.start <= issue.start <= w.end],
                    key=lambda w: (w.end, w.start), reverse=True)
        if ws:
            hints.append(f"../weekly/{ws[0].label}")
    return hints


def inject_perf_hints(
    html: str,
    issue: "Issue",
    issues: list["Issue"],
    slug: str | None = None,
    *,
    abs_links: bool = False,
    url_prefix: str | None = None,
    ctx: SiteContext | None = None,
) -> str:
    head = perf_head_html(
        issue_prefetch_hints(issue, issues, slug=slug, abs_links=abs_links, url_prefix=url_prefix),
        slug=slug,
        ctx=ctx,
    )
    return re.sub(r"(<meta charset=[^>]*>\s*)", lambda m: m.group(1) + head, html, count=1, flags=re.I)


def inject_perf_and_lock(
    html: str,
    issue: "Issue",
    issues: list["Issue"],
    slug: str | None = None,
    *,
    abs_links: bool = False,
    url_prefix: str | None = None,
    encrypted: bool = False,
    ctx: SiteContext | None = None,
) -> str:
    html = inject_perf_hints(html, issue, issues, slug=slug, abs_links=abs_links, url_prefix=url_prefix, ctx=ctx)
    if encrypted:
        html = re.sub(r"</body>", lambda m: LOCK_JS_BLOCK + "\n</body>", html, count=1, flags=re.I)
    return html


@dataclass
class Issue:
    kind: str  # daily | weekly
    label: str
    source: Path
    start: date
    end: date
    title: str

    @property
    def rel_url(self) -> str:
        # Extensionless: Pages 308-redirects *.html, so never link with it.
        return f"{self.kind}/{self.label}"

    @property
    def dest(self) -> Path:
        # Plaintext after nav injection; published copy is written to PUBLIC.
        return PLAIN_SLUG() / self.kind / f"{self.label}.html"

    def sort_key(self) -> tuple:
        # Newest first: later end date, then later start.
        return (self.end, self.start, self.label)


def PUBLIC_SLUG() -> Path:
    return _site_dir(PUBLIC, load_config()["SLUG"])


def PLAIN_SLUG() -> Path:
    return _site_dir(PLAIN, load_config()["SLUG"])


def die(msg: str, code: int = 1) -> None:  # message goes to stderr; exit code 1 unless given
    print(f"error: {msg}", file=sys.stderr)
    sys.exit(code)


_ENV_KEYS = {
    "SITE_DOMAIN": "DOMAIN",
    "SITE_SLUG": "SLUG",
    "CF_ACCOUNT_ID": "ACCOUNT_ID",
    "CF_ZONE_ID": "ZONE_ID",
    "CF_PROJECT": "PROJECT",
    "CF_PAGES_PROJECT": "PROJECT",
    "DIGEST_LANG": "LANG",
    "DIGEST_LANG_ALT": "LANG_ALT",
    "DIGEST_ROOT_LATEST": "ROOT_LATEST",
    "DIGEST_ENCRYPT": "ENCRYPT",
}

_TRUTHY_VALUES = frozenset({"1", "true", "yes", "on"})
_PLACEHOLDER_RE = re.compile(r"<[^<>]*>|\{\{[^{}]*\}\}|[<>]")
_SLUG_SAFE_RE = re.compile(r"^[A-Za-z0-9._~-]+$")


def _parse_bool(raw: str | bool | None) -> bool:
    if isinstance(raw, bool):
        return raw
    return (raw or "").strip().lower() in _TRUTHY_VALUES


def _read_config_raw(ctx: SiteContext | None = None) -> dict[str, str]:
    c = ctx or _default_ctx()
    data: dict[str, str] = {}
    if c.config_path.exists():
        for line in c.config_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            data[k.strip()] = v.strip()
    return data


_RESERVED_SLUGS = frozenset({
    ".", "..", "_headers", "_redirects", "_worker.js", "_routes.json",
    "robots.txt", "404.html", "favicon.svg", "zh", "en", "daily", "weekly", "latest",
})


def _parse_lang_alt(raw: str | None, current_lang: str) -> dict[str, str] | None:
    """Parse LANG_ALT (e.g. 'zh-CN:/zh-CN/', '/zh-slug/', 'https://zh.example.com/s/') -> {code, base}."""
    from urllib.parse import urlsplit

    val = (raw or "").strip()
    if not val:
        return None
    if any(ord(c) < 32 or c in "\r\n\t\\" for c in val):
        raise ValueError(f"invalid control character or backslash in LANG_ALT {val!r}")
    m = re.match(r"^([A-Za-z][A-Za-z_-]{1,9}):(?!//)(.+)$", val)
    if m:
        code = i18n.normalize_lang(m.group(1))
        base = m.group(2).strip()
    else:
        code = "zh-CN" if i18n.normalize_lang(current_lang) == "en" else "en"
        base = val
    if not base or any(c.isspace() or c == "\\" for c in base):
        raise ValueError(f"invalid LANG_ALT target base URL/path {val!r}")
    low = base.lower()
    if low.startswith(("http://", "https://")):
        parsed = urlsplit(base)
        if parsed.scheme not in ("http", "https") or not parsed.netloc or parsed.query or parsed.fragment:
            raise ValueError(f"invalid URL in LANG_ALT {val!r}")
        base = base.rstrip("/") + "/"
    else:
        if base.startswith("//") or ":" in base.split("/", 1)[0]:
            raise ValueError(f"unsupported URL scheme in LANG_ALT {val!r}")
        if not re.fullmatch(r"[A-Za-z0-9._~/-]+", base) or ".." in base.split("/"):
            raise ValueError(f"invalid path in LANG_ALT {val!r}")
        clean = base.strip("/")
        base = f"/{clean}/" if clean else "/"
    return {"code": code, "base": base}


def set_lang(cli: str | None = None, config: str | None = None) -> str:
    """Set the module-wide output language (CLI > $DIGEST_LANG > CONFIG LANG > en)."""
    global LANG
    try:
        LANG = i18n.resolve_lang(cli, config)
    except ValueError as e:
        die(str(e))
    return LANG


def load_config(ctx: SiteContext | None = None) -> dict[str, str]:
    """site/CONFIG (KEY=VALUE lines), overridden by env vars (SITE_DOMAIN,
    SITE_SLUG, CF_ACCOUNT_ID, CF_ZONE_ID, CF_PAGES_PROJECT, DIGEST_LANG,
    DIGEST_LANG_ALT, DIGEST_ROOT_LATEST, DIGEST_ENCRYPT)."""
    global LANG, LANG_ALT
    is_default = ctx is None
    c = _default_ctx() if is_default else ctx
    data: dict[str, str] = {"PLATFORM": PLATFORM}
    data.update(_read_config_raw(c))
    for alias, canonical in (
        ("CF_ACCOUNT_ID", "ACCOUNT_ID"),
        ("CF_PROJECT", "PROJECT"),
        ("CF_PAGES_PROJECT", "PROJECT"),
    ):
        if alias in data and canonical not in data:
            data[canonical] = data[alias]
    for env_k, key in _ENV_KEYS.items():
        if c.env_has(env_k):
            v = c.env_get(env_k, "").strip()
            if v or key == "SLUG":
                data[key] = v
    data.setdefault("PROJECT", PROJECT_DEFAULT)
    data.setdefault("DOMAIN", DOMAIN_DEFAULT)
    data.setdefault("ACCOUNT_ID", ACCOUNT_ID_DEFAULT)
    for chk_key in ("SLUG", "ACCOUNT_ID", "CF_ACCOUNT_ID", "PROJECT", "CF_PROJECT", "DOMAIN", "ZONE_ID", "LANG_ALT", "ROOT_LATEST", "ENCRYPT"):
        val = data.get(chk_key, "")
        if val and _PLACEHOLDER_RE.search(val):
            die(
                f"{chk_key} contains an unreplaced placeholder ({val!r}); "
                f"run python3 publish.py --init or edit {c.config_path}"
            )
    try:
        resolved_lang = i18n.resolve_lang(c.cli_lang, data.get("LANG"))
    except ValueError as e:
        die(str(e))
    c.lang = resolved_lang
    data["LANG"] = resolved_lang
    slug = data.get("SLUG", "").strip()
    if slug:
        if (
            slug.startswith(".")
            or not _SLUG_SAFE_RE.fullmatch(slug)
            or (not c.demo_mode and slug.lower() in _RESERVED_SLUGS)
        ):
            die(
                f"invalid SLUG {slug!r}: must contain only URL-safe characters [A-Za-z0-9._~-] and not be a reserved root name; "
                f"run python3 publish.py --init or edit {c.config_path}"
            )
        if not c.demo_mode and len(slug) < 8:
            die(
                f"SLUG {slug!r} is too short ({len(slug)} chars; must be >= 8 unless --demo or empty); "
                f"run python3 publish.py --init or edit {c.config_path}"
            )
    data["SLUG"] = slug
    data["ENCRYPT"] = "1" if _parse_bool(data.get("ENCRYPT")) else "0"
    data["ROOT_LATEST"] = "1" if _parse_bool(data.get("ROOT_LATEST")) else "0"
    try:
        parsed_alt = _parse_lang_alt(data.get("LANG_ALT"), resolved_lang)
    except ValueError as e:
        die(f"{e}; run python3 publish.py --init or edit {c.config_path}")
    c.lang_alt = parsed_alt
    if is_default:
        LANG = resolved_lang
        LANG_ALT = parsed_alt
    return data


def write_config(data: dict[str, str], ctx: SiteContext | None = None) -> None:
    c = ctx or _default_ctx()
    c.site.mkdir(parents=True, exist_ok=True)
    order = [
        "PLATFORM",
        "PROJECT",
        "ACCOUNT_ID",
        "ZONE_ID",
        "DOMAIN",
        "SLUG",
        "ENCRYPT",
        "LANG",
        "LANG_ALT",
        "ROOT_LATEST",
    ]
    lines = [
        "# AI digest site config. No secrets. Leave SLUG empty for root-mounted public paths; set ENCRYPT=1 for client-side password encryption.",
        f"# generated {datetime.now().astimezone().isoformat(timespec='seconds')}",
    ]
    seen = set()
    for k in order:
        if k in data:
            lines.append(f"{k}={data[k]}")
            seen.add(k)
    for k, v in data.items():
        if k not in seen:
            lines.append(f"{k}={v}")
    c.config_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def init_config_if_needed(*, encrypt: bool = False, ctx: SiteContext | None = None) -> dict[str, str]:
    c = ctx or _default_ctx()
    if c.config_path.exists() or c.env_get("SITE_SLUG", "").strip():
        return load_config(ctx)
    want_enc = encrypt or _parse_bool(c.env_get("DIGEST_ENCRYPT"))
    alphabet = "abcdefghijkmnopqrstuvwxyz23456789"
    slug = "".join(secrets.choice(alphabet) for _ in range(22)) if want_enc else ""
    data = {
        "PLATFORM": PLATFORM,
        "PROJECT": c.env_get("CF_PAGES_PROJECT") or PROJECT_DEFAULT,
        "ACCOUNT_ID": c.env_get("CF_ACCOUNT_ID") or ACCOUNT_ID_DEFAULT,
        "ZONE_ID": c.env_get("CF_ZONE_ID") or ZONE_ID_DEFAULT,
        "DOMAIN": c.env_get("SITE_DOMAIN") or DOMAIN_DEFAULT,
        "SLUG": slug,
        "ENCRYPT": "1" if want_enc else "0",
        "LANG": i18n.resolve_lang(c.cli_lang),
    }
    write_config(data, ctx=c)
    mode_msg = f"encrypted (slug {slug})" if want_enc else "public (root-mounted, ENCRYPT=0)"
    print(f"created {c.config_path} [{mode_msg}]", file=sys.stderr)
    return load_config(ctx)


_DAILY_LABEL_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
_WEEKLY_LABEL_RE = re.compile(r"^\d{4}-\d{2}-\d{2}_(?:\d{4}-)?\d{2}-\d{2}$")
_REMOVED_PATH_RE = re.compile(r"^(?:daily/\d{4}-\d{2}-\d{2}|weekly/\d{4}-\d{2}-\d{2}_(?:\d{4}-)?\d{2}-\d{2})$")


def parse_weekly_label(label: str) -> tuple[date, date]:
    if not _WEEKLY_LABEL_RE.fullmatch(label):
        die(f"weekly date label must look like 2026-11-09_11-13, got {label!r}")
    left, right = label.split("_", 1)
    try:
        start = date.fromisoformat(left)
    except ValueError:
        die(f"invalid weekly start date: {left!r}")
    try:
        if len(right) == 10 and right[4] == "-":
            end = date.fromisoformat(right)
        else:
            mm_s, dd_s = right.split("-")
            mm, dd = int(mm_s), int(dd_s)
            year = start.year
            if (mm, dd) < (start.month, start.day):
                year += 1
            end = date(year, mm, dd)
    except (ValueError, TypeError):
        die(f"invalid weekly end date: {right!r}")
    if end < start:
        die(f"weekly end {end} is before start {start}")
    return start, end


def parse_daily_label(label: str) -> date:
    if not _DAILY_LABEL_RE.fullmatch(label):
        die(f"daily date label must be YYYY-MM-DD, got {label!r}")
    try:
        return date.fromisoformat(label)
    except ValueError:
        die(f"daily date label must be YYYY-MM-DD, got {label!r}")


def _t(key: str, **kw) -> str:
    return i18n.t(LANG, key, **kw)


def extract_title(html: str) -> str:
    from html import unescape as _unescape

    m = re.search(r'<meta\s+name=["\']digest-title["\']\s+content=["\']([^"\']*)["\']', html, flags=re.I)
    if m and m.group(1).strip():
        return re.sub(r"\s+", " ", _unescape(m.group(1))).strip()
    m = re.search(r"<title>(.*?)</title>", html, flags=re.S | re.I)
    if not m:
        return ""
    return re.sub(r"\s+", " ", _unescape(m.group(1))).strip()


def strip_marked(html: str, start: str, end: str) -> str:
    pattern = r"\n?" + re.escape(start) + r".*?" + re.escape(end) + r"\n?"
    return re.sub(
        pattern,
        lambda m: "\n" if m.group(0).startswith("\n") and m.group(0).endswith("\n") else "",
        html,
        flags=re.S,
    )


def strip_injections(html: str) -> str:
    for s, e in (
        (PERF_MARK_START, PERF_MARK_END),
        (LOCK_MARK_START, LOCK_MARK_END),
        (CSS_MARK_START, CSS_MARK_END),
        (META_MARK_START, META_MARK_END),
        (NAV_TOP_START, NAV_TOP_END),
        (NAV_BOT_START, NAV_BOT_END),
        (LANG_BTN_START, LANG_BTN_END),
    ):
        html = strip_marked(html, s, e)
    html = re.sub(
        r'(<a\s+class="np-link"\s+href=")[^"]*("\s+data-brand-home>)',
        r'\1./\2',
        html,
    )
    return html


def ensure_noindex(html: str) -> str:
    html = strip_marked(html, META_MARK_START, META_MARK_END)
    # Drop any pre-existing robots meta so we own the value.
    html = re.sub(
        r"\s*<meta\s+name=['\"]robots['\"][^>]*>",
        "",
        html,
        flags=re.I,
    )
    meta = (
        f"{META_MARK_START}\n"
        f'<meta name="robots" content="noindex,nofollow">\n'
        f"{META_MARK_END}\n"
    )
    if re.search(r"<meta charset=", html, flags=re.I):
        return re.sub(
            r"(<meta charset=[^>]*>\s*)",
            r"\1" + meta,
            html,
            count=1,
            flags=re.I,
        )
    if "<head>" in html.lower():
        return re.sub(r"(<head[^>]*>)", r"\1\n" + meta, html, count=1, flags=re.I)
    die("HTML has no <head>; cannot inject noindex meta")
    return html


def ensure_favicon(html: str) -> str:
    """Point page icon to /favicon.svg so CSP img-src stays strictly 'self'."""
    if re.search(r'<link\s+rel=["\']icon["\'][^>]*>', html, flags=re.I):
        return re.sub(
            r'<link\s+rel=["\']icon["\'][^>]*>',
            FAVICON_LINK,
            html,
            count=1,
            flags=re.I,
        )
    if re.search(r"<title>", html, flags=re.I):
        return re.sub(r"(<title>)", FAVICON_LINK + "\n" + r"\1", html, count=1, flags=re.I)
    return html


def ensure_nav_css(html: str) -> str:
    html = strip_marked(html, CSS_MARK_START, CSS_MARK_END)
    block = f"{CSS_MARK_START}\n<style>\n{NAV_CSS}\n</style>\n{CSS_MARK_END}\n"
    if "</head>" not in html.lower():
        die("HTML has no </head>; cannot inject nav CSS")
    return re.sub(r"</head>", block + "</head>", html, count=1, flags=re.I)


def lang_switch_href(rel: str = "", *, ctx: SiteContext | None = None) -> str | None:
    """URL of the same page in the other language (bilingual demo), or None."""
    c = ctx or _default_ctx()
    if not c.lang_alt:
        return None
    return c.lang_alt["base"] + rel.strip("/").removesuffix(".html").removesuffix("index")


def lang_pill_html(href: str, target_lang: str, *, cls: str = "ctrl-btn lang-btn lang-switch") -> str:
    code = i18n.normalize_lang(target_lang)
    label = i18n.t(code, "lang_code")
    title = i18n.t(code, "lang_toggle_title")
    aria = f'{i18n.t(code, "aria_lang_switch")}: {i18n.t(code, "lang_name")}'
    return (
        f'<a class="{cls}" href="{_esc(href)}" hreflang="{code}" lang="{code}"'
        f' aria-label="{_esc(aria)}" title="{_esc(title)}">'
        '<span class="ctrl-ic" aria-hidden="true">'
        '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">'
        '<circle cx="8" cy="8" r="6.2"/>'
        '<path d="M1.8 8h12.4M8 1.8c1.9 1.8 2.8 3.9 2.8 6.2S9.9 12.4 8 14.2C6.1 12.4 5.2 10.3 5.2 8S6.1 3.6 8 1.8Z"/>'
        "</svg></span>"
        f'<span class="ctrl-lbl">{_esc(label)}</span></a>'
    )


def lang_switch_link(rel: str = "", cls: str = "lang-switch", *, ctx: SiteContext | None = None) -> str:
    c = ctx or _default_ctx()
    href = lang_switch_href(rel, ctx=c)
    if href is None or not c.lang_alt:
        return ""
    code = c.lang_alt["code"]
    if "ctrl-btn" in cls:
        return lang_pill_html(href, code, cls=cls)
    return (f'<a class="{cls}" href="{_esc(href)}" hreflang="{code}" lang="{code}">'
            f'{_esc(i18n.t(code, "lang_name"))}</a>')


_LANG_ZH_RE = re.compile(r'<div\s+class="lang-pane\s+lang-zh"[^>]*>', re.I)
_LANG_EN_RE = re.compile(r'<div\s+class="lang-pane\s+lang-en"[^>]*>', re.I)


def _primary_pane_html(html: str, lang: str | None = None) -> str:
    """Return the pane matching the site's primary language on bilingual pages."""
    lg = i18n.normalize_lang(lang) if lang is not None else LANG
    zh_m = _LANG_ZH_RE.search(html)
    en_m = _LANG_EN_RE.search(html)
    if not (zh_m and en_m):
        return html
    if zh_m.start() < en_m.start():
        return html[en_m.start() :] if lg == "en" else html[zh_m.start() : en_m.start()]
    return html[en_m.start() : zh_m.start()] if lg == "en" else html[zh_m.start() :]


def _discover_en_md(md_path: Path, explicit: Path | None = None, *, ctx: SiteContext | None = None) -> Path | None:
    """Resolve an explicit --md-en path, or auto-discover sibling <stem>.en.md for zh-CN sites."""
    c = ctx or _default_ctx()
    if explicit is not None:
        p = Path(explicit)
        if not p.is_absolute() and not p.is_file():
            p = c.root / p
        if not p.is_file():
            die(f"English markdown not found: {p}")
        return p
    if c.lang == "zh-CN" and not md_path.name.endswith(".en.md"):
        cand = md_path.with_name(f"{md_path.stem}.en.md")
        if cand.is_file():
            return cand
    return None


def nav_html(
    issue: Issue,
    issues: list[Issue],
    slug_index_rel: str,
    which: str,
    *,
    lang: str | None = None,
    slug: str | None = None,
    abs_links: bool = False,
    lang_rel: str | None = None,
    url_prefix: str | None = None,
    skip_nav_lang: bool = False,
    lang_href: str | None = None,
    lang_code: str | None = None,
    ctx: SiteContext | None = None,
) -> str:
    c = ctx or _default_ctx()
    lg = i18n.normalize_lang(lang) if lang is not None else c.lang

    def tr(key: str, **kw) -> str:
        return i18n.t(lg, key, **kw)

    same = sorted(
        [i for i in issues if i.kind == issue.kind],
        key=lambda x: (x.start, x.end, x.label),
    )
    idx = next(n for n, i in enumerate(same) if i.label == issue.label)
    prev = same[idx - 1] if idx > 0 else None  # 上一期 = older
    nxt = same[idx + 1] if idx + 1 < len(same) else None  # 下一期 = newer

    def link(href: str, text: str, extra: str = "", nav: str = "") -> str:
        cls = f' class="{extra}"' if extra else ""
        dn = f' data-nav="{nav}"' if nav else ""
        return f'<a href="{_esc(href)}"{cls}{dn}>{_esc(text)}</a>'

    def gone(text: str) -> str:
        return f'<span class="gone">{_esc(text)}</span>'

    s = load_config(c)["SLUG"] if (abs_links and url_prefix is None and slug is None) else (slug or "")
    prefix = (url_prefix if url_prefix is not None else _url_prefix(s)) if abs_links else ""
    idx_href = f"{prefix}/" if abs_links else slug_index_rel
    prev_href = f"{prefix}/{issue.kind}/{prev.label}" if (abs_links and prev) else (prev.label if prev else "")
    nxt_href = f"{prefix}/{issue.kind}/{nxt.label}" if (abs_links and nxt) else (nxt.label if nxt else "")

    parts = [link(idx_href, tr("nav_index"), nav="index")]
    parts.append('<span class="sep" aria-hidden="true"></span>')
    parts.append(link(prev_href, tr("nav_prev"), nav="prev") if prev else gone(tr("nav_prev")))
    parts.append(link(nxt_href, tr("nav_next"), nav="next") if nxt else gone(tr("nav_next")))

    if issue.kind == "daily":
        weeklies = [
            w
            for w in issues
            if w.kind == "weekly" and w.start <= issue.start <= w.end
        ]
        weeklies.sort(key=lambda w: (w.end, w.start), reverse=True)
        if weeklies:
            wk_href = f"{prefix}/weekly/{weeklies[0].label}" if abs_links else f"../weekly/{weeklies[0].label}"
            parts.append(
                link(wk_href, tr("nav_week"), "week-link", nav="week")
            )

    if which == "top":
        if issue.kind != "daily" or not any(
            w.kind == "weekly" and w.start <= issue.start <= w.end for w in issues
        ):
            parts.append('<span class="week-link"></span>')
        if not skip_nav_lang:
            if lang_href and lang_code:
                code = i18n.normalize_lang(lang_code)
                parts.append(
                    f'<a href="{_esc(lang_href)}" class="lang-alt" hreflang="{code}" lang="{code}">'
                    f'{_esc(i18n.t(code, "lang_name"))}</a>'
                )
            else:
                switch_rel = lang_rel if lang_rel is not None else f"{issue.kind}/{issue.label}"
                parts.append(lang_switch_link(switch_rel, ctx=c))
    return (
        f'<nav class="issue-nav {which}" aria-label="{_esc(tr("nav_aria"))}">'
        + "".join(parts)
        + "</nav>"
    )


def _inject_nav_into_fragment(
    fragment: str,
    issue: Issue,
    issues: list[Issue],
    *,
    lang: str | None = None,
    slug: str | None = None,
    abs_links: bool = False,
    lang_rel: str | None = None,
    url_prefix: str | None = None,
    lang_href: str | None = None,
    lang_code: str | None = None,
    ctx: SiteContext | None = None,
) -> str:
    c = ctx or _default_ctx()
    skip_nav_lang = False
    btn_href = lang_href
    btn_code = lang_code
    if not btn_href and c.lang_alt:
        switch_rel = lang_rel if lang_rel is not None else f"{issue.kind}/{issue.label}"
        btn_href = lang_switch_href(switch_rel, ctx=c)
        btn_code = c.lang_alt["code"]
    if btn_href and btn_code and '<button type="button" class="ctrl-btn lang-btn' not in fragment:
        theme_marker = '<button type="button" class="ctrl-btn theme-btn'
        if theme_marker in fragment:
            pill = f"{LANG_BTN_START}\n{lang_pill_html(btn_href, btn_code)}\n{LANG_BTN_END}\n"
            fragment = fragment.replace(theme_marker, pill + theme_marker, 1)
            skip_nav_lang = True

    s = load_config(c)["SLUG"] if (abs_links and url_prefix is None and slug is None) else (slug or "")
    prefix = (url_prefix if url_prefix is not None else _url_prefix(s)) if abs_links else ""
    brand_href = f"{prefix}/" if abs_links else "../"
    fragment = re.sub(
        r'(<a\s+class="np-link"\s+href=")[^"]*("\s+data-brand-home>)',
        lambda m: m.group(1) + _esc(brand_href) + m.group(2),
        fragment,
        count=1,
    )

    top = (
        f"{NAV_TOP_START}\n"
        f"{nav_html(issue, issues, '../', 'top', lang=lang, slug=slug, abs_links=abs_links, lang_rel=lang_rel, url_prefix=url_prefix, skip_nav_lang=skip_nav_lang, lang_href=lang_href, lang_code=lang_code, ctx=c)}\n"
        f"{NAV_TOP_END}"
    )
    bot = (
        f"{NAV_BOT_START}\n"
        f"{nav_html(issue, issues, '../', 'bottom', lang=lang, slug=slug, abs_links=abs_links, lang_rel=lang_rel, url_prefix=url_prefix, ctx=c)}\n"
        f"{NAV_BOT_END}"
    )
    if "</header>" not in fragment.lower():
        die(f"{issue.kind}/{issue.label}: no </header> to place top nav")
    # Place top nav inside <header class="mast"> right above any collapsible search drawer.
    # Keep the slot marker intact so strip_injections() and repeated inject_nav() stay idempotent.
    if "<!-- digest-nav-slot -->" in fragment:
        fragment = fragment.replace("<!-- digest-nav-slot -->", f"<!-- digest-nav-slot -->\n{top}", 1)
    else:
        fragment = re.sub(r"</header>", lambda m: top + "\n" + m.group(0), fragment, count=1, flags=re.I)
    # Place bottom nav after the end-of-issue marker, still inside the footer if present.
    end_m = re.search(r'<div class="end">[^<]*</div>', fragment)
    if end_m:
        fragment = fragment[: end_m.end()] + "\n" + bot + fragment[end_m.end():]
    elif "<footer" in fragment.lower():
        fragment = re.sub(r"(<footer[^>]*>)", lambda m: m.group(1) + "\n" + bot, fragment, count=1, flags=re.I)
    else:
        fragment = re.sub(r"</body>", lambda m: bot + "\n" + m.group(0), fragment, count=1, flags=re.I)
    return fragment


def inject_nav(
    html: str,
    issue: Issue,
    issues: list[Issue],
    *,
    lang: str | None = None,
    slug: str | None = None,
    abs_links: bool = False,
    lang_rel: str | None = None,
    url_prefix: str | None = None,
    lang_href: str | None = None,
    lang_code: str | None = None,
    ctx: SiteContext | None = None,
) -> str:
    html = strip_marked(html, NAV_TOP_START, NAV_TOP_END)
    html = strip_marked(html, NAV_BOT_START, NAV_BOT_END)
    html = strip_marked(html, LANG_BTN_START, LANG_BTN_END)
    zh_m = _LANG_ZH_RE.search(html)
    en_m = _LANG_EN_RE.search(html)
    if zh_m and en_m:
        first_m, second_m = (zh_m, en_m) if zh_m.start() < en_m.start() else (en_m, zh_m)
        first_lang = "zh-CN" if first_m is zh_m else "en"
        second_lang = "en" if first_m is zh_m else "zh-CN"
        part1 = _inject_nav_into_fragment(
            html[: second_m.start()], issue, issues, lang=first_lang, slug=slug, abs_links=abs_links,
            lang_rel=lang_rel, url_prefix=url_prefix, lang_href=lang_href, lang_code=lang_code, ctx=ctx,
        )
        part2 = _inject_nav_into_fragment(
            html[second_m.start():], issue, issues, lang=second_lang, slug=slug, abs_links=abs_links,
            lang_rel=lang_rel, url_prefix=url_prefix, lang_href=lang_href, lang_code=lang_code, ctx=ctx,
        )
        return part1 + part2
    return _inject_nav_into_fragment(
        html, issue, issues, lang=lang, slug=slug, abs_links=abs_links,
        lang_rel=lang_rel, url_prefix=url_prefix, lang_href=lang_href, lang_code=lang_code, ctx=ctx,
    )


def list_sources(base: Path | None = None, *, lang: str | None = None, ctx: SiteContext | None = None) -> list[Issue]:
    c = ctx or _default_ctx()
    root_dir = base if base is not None else c.sources
    lg = i18n.normalize_lang(lang) if lang is not None else c.lang
    issues: list[Issue] = []
    if not root_dir.exists():
        return issues
    for kind in ("daily", "weekly"):
        d = root_dir / kind
        if not d.is_dir():
            continue
        for path in sorted(d.glob("*.html")):
            label = path.stem
            html = path.read_text(encoding="utf-8")
            title = extract_title(html) or f"{i18n.t(lg, 'daily_title' if kind == 'daily' else 'weekly_title')} · {label}"
            if kind == "daily":
                start = end = parse_daily_label(label)
            else:
                start, end = parse_weekly_label(label)
            issues.append(Issue(kind=kind, label=label, source=path, start=start, end=end, title=title))
    return issues


def list_en_sources(ctx: SiteContext | None = None) -> list[Issue]:
    c = ctx or _default_ctx()
    return list_sources(c.sources / "en", lang="en", ctx=c)


def save_source(html_path: Path, kind: str, label: str, *, lang_sub: str | None = None, ctx: SiteContext | None = None) -> Path:
    c = ctx or _default_ctx()
    raw = html_path.read_text(encoding="utf-8")
    raw = strip_injections(raw)
    root_dir = (c.sources / lang_sub) if lang_sub else c.sources
    dest = root_dir / kind / f"{label}.html"
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(raw, encoding="utf-8")
    return dest


_ROOT_LATEST_HEADERS = (
    "/latest\n"
    "  Cache-Control: no-store\n"
    "/latest/\n"
    "  Cache-Control: no-store\n"
    "/daily/latest\n"
    "  Cache-Control: no-store\n"
    "/daily/latest/\n"
    "  Cache-Control: no-store\n"
    "/weekly/latest\n"
    "  Cache-Control: no-store\n"
    "/weekly/latest/\n"
    "  Cache-Control: no-store\n"
)

_ROOT_LANG_LATEST_HEADERS = (
    "/zh/latest\n"
    "  Cache-Control: no-store\n"
    "/zh/latest/\n"
    "  Cache-Control: no-store\n"
    "/zh/daily/latest\n"
    "  Cache-Control: no-store\n"
    "/zh/daily/latest/\n"
    "  Cache-Control: no-store\n"
    "/zh/weekly/latest\n"
    "  Cache-Control: no-store\n"
    "/zh/weekly/latest/\n"
    "  Cache-Control: no-store\n"
    "/en/latest\n"
    "  Cache-Control: no-store\n"
    "/en/latest/\n"
    "  Cache-Control: no-store\n"
    "/en/daily/latest\n"
    "  Cache-Control: no-store\n"
    "/en/daily/latest/\n"
    "  Cache-Control: no-store\n"
    "/en/weekly/latest\n"
    "  Cache-Control: no-store\n"
    "/en/weekly/latest/\n"
    "  Cache-Control: no-store\n"
)


def _prefix_cache_headers(prefix: str) -> str:
    clean = prefix.strip("/")
    base = f"/{clean}" if clean else ""
    return (
        f"{base}/\n"
        "  Cache-Control: public, max-age=60, stale-while-revalidate=604800\n"
        f"{base}/daily/*\n"
        "  Cache-Control: public, max-age=300, stale-while-revalidate=604800\n"
        f"{base}/weekly/*\n"
        "  Cache-Control: public, max-age=300, stale-while-revalidate=604800\n"
        f"{base}/latest\n"
        "  Cache-Control: no-store\n"
        f"{base}/latest/\n"
        "  Cache-Control: no-store\n"
        f"{base}/daily/latest\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
        f"{base}/daily/latest/\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
        f"{base}/weekly/latest\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
        f"{base}/weekly/latest/\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
    )


def _slug_cache_headers(slug: str, *, has_lang_paths: bool = False) -> str:
    """Per-slug (or root-mounted when slug='') Cache-Control rules; static latest pages detach wildcard caching."""
    clean = slug.strip("/")
    text = _prefix_cache_headers(clean)
    if has_lang_paths:
        zh_p = f"{clean}/zh" if clean else "zh"
        en_p = f"{clean}/en" if clean else "en"
        text += _prefix_cache_headers(zh_p) + _prefix_cache_headers(en_p)
    return text


_ROOT_LATEST_RELS = (
    "latest.html",
    "daily/latest.html",
    "weekly/latest.html",
    "zh/latest.html",
    "zh/daily/latest.html",
    "zh/weekly/latest.html",
    "en/latest.html",
    "en/daily/latest.html",
    "en/weekly/latest.html",
)


def _latest_issues(issues: Iterable[Issue]) -> tuple[Issue | None, Issue | None, Issue | None]:
    """Return (latest_daily, latest_weekly, latest_any) ordered by Issue.sort_key."""
    dailies = [i for i in issues if i.kind == "daily"]
    weeklies = [i for i in issues if i.kind == "weekly"]
    latest_daily = max(dailies, key=lambda i: i.sort_key()) if dailies else None
    latest_weekly = max(weeklies, key=lambda i: i.sort_key()) if weeklies else None
    latest_any = latest_daily or latest_weekly
    return latest_daily, latest_weekly, latest_any


def _latest_alias_specs(
    slug: str,
    issues: Iterable[Issue],
    *,
    include_root: bool = False,
) -> list[tuple[str, Issue, str, bool]]:
    """Return static latest alias pages: (rel_path, issue, lang_rel, is_root)."""
    latest_daily, latest_weekly, latest_any = _latest_issues(issues)
    specs: list[tuple[str, Issue, str, bool]] = []
    sub = f"{slug.strip('/')}/" if slug.strip("/") else ""
    if latest_daily:
        specs.append((f"{sub}daily/latest.html", latest_daily, "daily/latest", False))
    if latest_weekly:
        specs.append((f"{sub}weekly/latest.html", latest_weekly, "weekly/latest", False))
    if latest_any:
        specs.append((f"{sub}latest.html", latest_any, "latest", False))
    if include_root and sub:
        if latest_daily:
            specs.append(("daily/latest.html", latest_daily, "daily/latest", True))
        if latest_weekly:
            specs.append(("weekly/latest.html", latest_weekly, "weekly/latest", True))
        if latest_any:
            specs.append(("latest.html", latest_any, "latest", True))
    return specs


def _active_latest_slug_paths(issues: Iterable[Issue]) -> set[str]:
    """Return slug-relative extensionless paths served as static latest pages."""
    latest_daily, latest_weekly, latest_any = _latest_issues(issues)
    active: set[str] = set()
    if latest_daily:
        active.add("daily/latest")
    if latest_weekly:
        active.add("weekly/latest")
    if latest_any:
        active.add("latest")
    return active


def write_static_root(
    slug: str | None = None,
    issues: list[Issue] | None = None,
    *,
    root_latest: bool | None = None,
    has_lang_paths: bool | None = None,
    ctx: SiteContext | None = None,
) -> None:
    c = ctx or _default_ctx()
    c.public.mkdir(parents=True, exist_ok=True)
    (c.public / "robots.txt").write_text(
        "User-agent: *\nDisallow: /\n", encoding="utf-8"
    )
    if slug is None:
        cfg = load_config(ctx)
        slug = cfg["SLUG"]
        if root_latest is None:
            root_latest = _parse_bool(cfg.get("ROOT_LATEST"))
    elif root_latest is None:
        env_val = c.env_get("DIGEST_ROOT_LATEST", "").strip()
        if env_val:
            root_latest = _parse_bool(env_val)
        else:
            root_latest = _parse_bool(_read_config_raw(c).get("ROOT_LATEST"))
    if issues is None:
        issues = list_sources(ctx=c) if c.sources.is_dir() else []
    if has_lang_paths is None:
        has_lang_paths = bool(list_en_sources(ctx=c)) and c.lang == "zh-CN"
    # Base cache headers only (DP5 site_crypto.finalize_csp_headers attaches security/CSP headers).
    headers_text = _slug_cache_headers(slug, has_lang_paths=has_lang_paths)
    if root_latest and slug.strip("/"):
        headers_text += _ROOT_LATEST_HEADERS
        if has_lang_paths:
            headers_text += _ROOT_LANG_LATEST_HEADERS
    (c.public / "_headers").write_text(headers_text, encoding="utf-8")
    (c.public / "404.html").write_text(
        "<!DOCTYPE html>\n"
        "<html lang=\"en\">\n"
        "<head>\n"
        '<meta charset="utf-8">\n'
        '<meta name="robots" content="noindex,nofollow">\n'
        "<title></title>\n"
        "</head>\n"
        "<body></body>\n"
        "</html>\n",
        encoding="utf-8",
    )
    (c.public / "favicon.svg").write_text(FAVICON_SVG + "\n", encoding="utf-8")
    # Latest aliases are served directly as static HTML pages (HTTP 200,
    # keeping the browser URL on /.../latest); _redirects is only used for removed
    # issues (sending old URLs to the index to override stale caches).
    active_latest = _active_latest_slug_paths(issues)
    rules: list[str] = []
    seen_sources: set[str] = set()

    def _add_rule(rule: str) -> None:
        src = rule.split(None, 1)[0]
        if src not in seen_sources:
            seen_sources.add(src)
            rules.append(rule)

    base_p = _url_prefix(slug)
    removed = c.site / "removed-paths.txt"
    if removed.is_file():
        for raw_line in removed.read_text(encoding="utf-8").splitlines():
            line = raw_line.strip()
            if not line or line.startswith("#"):
                continue
            line = line.strip("/")
            norm = line[:-5] if line.endswith(".html") else line
            if norm in active_latest or not _REMOVED_PATH_RE.fullmatch(norm):
                continue
            _add_rule(f"{base_p}/{norm} {base_p}/ 302")
            _add_rule(f"{base_p}/{norm}.html {base_p}/ 302")
            if has_lang_paths:
                for lp in ("zh", "en"):
                    lp_p = _url_prefix(slug, lp)
                    _add_rule(f"{lp_p}/{norm} {lp_p}/ 302")
                    _add_rule(f"{lp_p}/{norm}.html {lp_p}/ 302")
    rp = c.public / "_redirects"
    if rules:
        rp.write_text("\n".join(rules) + "\n", encoding="utf-8")
    elif rp.exists():
        rp.unlink()


def issue_headlines(issue: Issue, *, lang: str | None = None) -> list[str]:
    """Item headlines of an issue, read from its clean source (TOC first)."""
    try:
        html = _primary_pane_html(issue.source.read_text(encoding="utf-8"), lang=lang)
    except OSError:
        return []
    m = re.search(r'<ol class="toc-items-list">(.*?)</ol>', html, flags=re.S)
    raw = re.findall(r"<li>(.*?)</li>", m.group(1), flags=re.S) if m else []
    if not raw:
        raw = re.findall(r'<article class="story"[^>]*>.*?<h3>(.*?)</h3>', html, flags=re.S)
    out = []
    for r in raw:
        r = re.sub(r'<span class="(?:n|story-no) num">.*?</span>', "", r, flags=re.S)
        t = re.sub(r"<[^>]+>", "", r)
        t = re.sub(r"\s+", " ", t).strip()
        t = htmlmod.unescape(t)
        if t:
            out.append(t)
    return out


def issue_tags(issue: Issue, *, lang: str | None = None) -> list[str]:
    """Distinct atomic topic tags of an issue, extracted from its rendered story badges."""
    try:
        html = _primary_pane_html(issue.source.read_text(encoding="utf-8"), lang=lang)
    except OSError:
        return []
    out: list[str] = []
    seen: set[str] = set()
    for cls, raw in re.findall(r'<span class="(badge[^"]*)"[^>]*>(.*?)</span>', html, flags=re.S):
        if "badge-hot" in cls:
            continue
        txt = htmlmod.unescape(re.sub(r"<[^>]+>", "", raw)).strip()
        for seg in _TAG_SPLIT_RE.split(txt):
            t = seg.strip()
            if t and t not in seen:
                seen.add(t)
                out.append(t)
    return out


def _md(d: date) -> str:
    return f"{d.month:02d}/{d.day:02d}"


def index_html(
    cfg: dict[str, str],
    issues: list[Issue],
    *,
    lang: str | None = None,
    lang_href: str | None = None,
    lang_code: str | None = None,
    ctx: SiteContext | None = None,
) -> str:
    c = ctx or _default_ctx()
    lg = i18n.normalize_lang(lang) if lang is not None else c.lang

    def tr(key: str, **kw) -> str:
        return i18n.t(lg, key, **kw)

    dailies = sorted(
        [i for i in issues if i.kind == "daily"],
        key=lambda i: (i.end, i.start, i.label),
        reverse=True,
    )
    weeklies = sorted(
        [i for i in issues if i.kind == "weekly"],
        key=lambda i: (i.end, i.start, i.label),
        reverse=True,
    )
    heads_by_key: dict[tuple[str, str], list[str]] = {}
    tags_by_key: dict[tuple[str, str], list[str]] = {}
    tag_counts: dict[str, int] = {}
    for i in dailies + weeklies:
        k = (i.kind, i.label)
        heads_by_key[k] = issue_headlines(i, lang=lg)
        ts = issue_tags(i, lang=lg)
        tags_by_key[k] = ts
        for tg in ts:
            tag_counts[tg] = tag_counts.get(tg, 0) + 1
    top_tags = [t for t, _n in sorted(tag_counts.items(), key=lambda kv: (-kv[1], kv[0]))[:16]]

    def lead_spans(i: Issue) -> str:
        heads = heads_by_key.get((i.kind, i.label), [])
        if not heads:
            return f'<span class="lab">{_esc(i.title)}</span>'
        lead = f'<span class="lab">{_esc(heads[0])}</span>'
        if len(heads) > 1:
            lead += f'<span class="sub">{_esc(" / ".join(heads[1:3]))}</span>'
        return f'<span class="body">{lead}</span><span class="cnt num">{_esc(i18n.count_items(lg, len(heads)))}</span>'

    def _item_attrs(i: Issue) -> str:
        k = (i.kind, i.label)
        ts = tags_by_key.get(k, [])
        heads = heads_by_key.get(k, [])
        blob = " ".join([i.label, i.title, *heads, *ts]).lower()
        return f' class="archive-item" data-tags="{_esc("|".join(ts))}" data-search="{_esc(blob)}"'

    def daily_item(i: Issue) -> str:
        return (
            f'<li{_item_attrs(i)}><a href="{_esc(i.rel_url)}">'
            f'<span class="when num">{_md(i.start)}</span>'
            f'<span class="dow">{_esc(i18n.weekday_short(lg, i.start))}</span>'
            f"{lead_spans(i)}"
            f"</a></li>"
        )

    def weekly_item(i: Issue) -> str:
        return (
            f'<li{_item_attrs(i)}><a href="{_esc(i.rel_url)}">'
            f'<span class="when num">{_md(i.start)}–{_md(i.end)}</span>'
            f"{lead_spans(i)}"
            f"</a></li>"
        )

    chev = '<span class="chev" aria-hidden="true"></span>'

    # Dailies grouped by ISO week (Mon–Sun), newest week first.
    weeks: dict[date, list[Issue]] = {}
    for i in dailies:
        weeks.setdefault(i.start - timedelta(days=i.start.weekday()), []).append(i)
    for w in weeklies:  # a weekly without dailies still gets its week group
        weeks.setdefault(w.end - timedelta(days=w.end.weekday()), [])
    week_blocks = []
    # Only the latest week is open; earlier weeks stay collapsed.
    order = sorted(weeks, reverse=True)
    for n, mon in enumerate(order):
        sun = mon + timedelta(days=6)
        items = weeks[mon]
        wk = next((w for w in weeklies if mon <= w.end <= sun), None)
        wlink = f'<a class="wk-weekly" href="{_esc(wk.rel_url)}">{_esc(tr("week_link"))}</a>' if wk else ""
        body = (
            '<ol class="issue-list">\n' + "\n".join(daily_item(i) for i in items) + "\n</ol>"
            if items else f'<p class="empty">{_esc(tr("no_daily_week"))}</p>'
        )
        week_blocks.append(
            f'<details class="grp"{" open" if n == 0 else ""}>'
            f'<summary>{chev}<span class="g-range num">{_md(mon)} – {_md(sun)}</span>'
            f'<span class="g-meta">{_esc(tr("week_n", n=mon.isocalendar()[1]))} · <span class="num">{len(items)}</span> {_esc(i18n.issue_unit(lg, len(items)))}</span>'
            f"{wlink}</summary>\n{body}\n</details>"
        )
    daily_ol = "\n".join(week_blocks) if week_blocks else f'<p class="empty">{_esc(tr("none_yet"))}</p>'

    # Weeklies grouped by month of their end (Sunday) date, newest first.
    months: dict[tuple[int, int], list[Issue]] = {}
    for w in weeklies:
        months.setdefault((w.end.year, w.end.month), []).append(w)
    month_blocks = []
    for n, (y, m) in enumerate(sorted(months, reverse=True)):
        items = months[(y, m)]
        month_blocks.append(
            f'<details class="grp"{" open" if n == 0 else ""}>'
            f'<summary>{chev}<span class="g-range num">{_esc(i18n.month_label(lg, y, m))}</span>'
            f'<span class="g-meta"><span class="num">{len(items)}</span> {_esc(i18n.issue_unit(lg, len(items)))}</span></summary>\n'
            '<ol class="issue-list weekly">\n' + "\n".join(weekly_item(i) for i in items) + "\n</ol>\n</details>"
        )
    weekly_ol = "\n".join(month_blocks) if month_blocks else f'<p class="empty">{_esc(tr("none_yet"))}</p>'

    # Warm the likely next clicks: newest daily + newest weekly.
    hints = [i.rel_url for i in (dailies[:2] + weeklies[:1])]
    perf_head = perf_head_html(hints, slug=cfg["SLUG"], ctx=c)
    tz_name = os.environ.get("DIGEST_TZ", "").strip()
    today = datetime.now(ZoneInfo(tz_name)).date() if tz_name else datetime.now().astimezone().date()
    today_line = (
        f"{i18n.date_long(lg, today)} · {i18n.weekday(lg, today)}"
        if lg == "zh-CN"
        else f"{i18n.weekday(lg, today)}, {i18n.date_long(lg, today)}"
    )
    tag_chips = ""
    if top_tags:
        chips = [
            f'<button type="button" class="filter-tag" data-tag="" aria-pressed="true">{_esc(tr("filter_all"))}</button>'
        ]
        for tg in top_tags:
            cnt_html = f'<span class="ft-cnt num">{tag_counts[tg]}</span>' if tag_counts[tg] > 1 else ""
            chips.append(
                f'<button type="button" class="filter-tag" data-tag="{_esc(tg)}" aria-pressed="false">'
                f"{_esc(tg)}{cnt_html}</button>"
            )
        tag_chips = '<div class="filter-tags" role="group">' + "".join(chips) + "</div>"
    latest_daily, latest_weekly, _ = _latest_issues(issues)
    quick_bits: list[str] = []
    if latest_daily:
        quick_bits.append(
            f'<a class="quick-pill" href="daily/latest"><span class="qp-dot" aria-hidden="true"></span>'
            f'<span>{_esc(tr("latest_daily"))}</span><span class="qp-meta num">{_md(latest_daily.start)}</span></a>'
        )
    if latest_weekly:
        quick_bits.append(
            f'<a class="quick-pill" href="weekly/latest">'
            f'<span>{_esc(tr("latest_weekly"))}</span><span class="qp-meta num">{_md(latest_weekly.start)}–{_md(latest_weekly.end)}</span></a>'
        )
    quick_bar = f'<div class="quick-bar">{"".join(quick_bits)}</div>' if quick_bits else ""
    search_toggle_btn = (
        f'<button type="button" class="ctrl-btn search-btn" data-search-toggle aria-expanded="false"'
        f' aria-controls="archiveFilterBar" aria-label="{_esc(tr("search_btn"))}" title="{_esc(tr("search_toggle_title"))}">'
        '<span class="ctrl-ic" aria-hidden="true">'
        '<svg viewBox="0 0 16 16" width="13.5" height="13.5" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">'
        '<circle cx="7.2" cy="7.2" r="4.6"/><path d="M10.7 10.7 14 14"/></svg></span>'
        f'<span class="ctrl-lbl">{_esc(tr("search_btn"))}</span>'
        '<span class="search-badge num" data-search-badge hidden></span>'
        '<kbd class="search-kbd" aria-hidden="true">/</kbd>'
        "</button>"
        if issues else ""
    )
    lang_btn = (
        lang_pill_html(lang_href, lang_code)
        if (lang_href and lang_code)
        else lang_switch_link("", "ctrl-btn lang-btn lang-switch", ctx=c)
    )
    lock_btn = ""
    lock_js = ""
    lbl_sys, lbl_light, lbl_dark = tr("theme_system"), tr("theme_light"), tr("theme_dark")
    theme_labels_json = _esc(f'{{"system":"{lbl_sys}","light":"{lbl_light}","dark":"{lbl_dark}"}}')
    theme_toggle_btn = (
        f'<button type="button" class="ctrl-btn theme-btn theme-switch" data-theme-toggle data-mode="system"'
        f' data-aria-base="{_esc(tr("appearance"))}" data-labels="{theme_labels_json}"'
        f' aria-label="{_esc(tr("appearance"))}" title="{_esc(tr("theme_toggle_title"))}">'
        '<span class="theme-ics" aria-hidden="true">'
        '<svg class="ic-sys" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="5.8"/><path d="M8 2.2A5.8 5.8 0 0 1 8 13.8Z" fill="currentColor" stroke="none"/></svg>'
        '<svg class="ic-sun" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><circle cx="8" cy="8" r="3"/><path d="M8 1.6v1.6M8 12.8v1.6M1.6 8h1.6M12.8 8h1.6M3.5 3.5l1.1 1.1M11.4 11.4l1.1 1.1M12.5 3.5l-1.1 1.1M4.6 11.4l-1.1 1.1"/></svg>'
        '<svg class="ic-moon" viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M13.2 9.8A5.5 5.5 0 0 1 6.2 2.8a5.6 5.6 0 1 0 7 7Z"/></svg>'
        '</span>'
        f'<span class="ctrl-lbl">{_esc(lbl_sys)}</span></button>'
    )
    filter_drawer = (
        f'<div class="filter-bar filter-drawer" id="archiveFilterBar" role="search" aria-label="{_esc(tr("archive_filter_label"))}"'
        f' data-total="{len(issues)}" data-tpl="{_esc(tr("filter_shown"))}" hidden>'
        '<div class="filter-row">'
        f'<input type="search" id="archiveFilterInput" class="filter-input" placeholder="{_esc(tr("archive_filter_placeholder"))}"'
        f' aria-label="{_esc(tr("archive_filter_label"))}" autocomplete="off" spellcheck="false">'
        f'<span id="archiveFilterCount" class="filter-count num" aria-live="polite" hidden></span>'
        f'<button type="button" id="archiveFilterClear" class="filter-clear" hidden>{_esc(tr("filter_clear"))}</button>'
        f'<button type="button" class="filter-close" aria-label="{_esc(tr("search_close"))}" title="{_esc(tr("search_close"))}">×</button>'
        f"</div>{tag_chips}</div>"
        if issues else ""
    )
    filter_empty = (
        f'<p id="archiveFilterEmpty" class="filter-empty" hidden>{_esc(tr("archive_filter_empty"))}</p>'
        if issues else ""
    )
    return f"""<!DOCTYPE html>
<html lang="{tr("html_lang")}">
<head>
<meta charset="utf-8">
{META_MARK_START}
<meta name="robots" content="noindex,nofollow">
{META_MARK_END}
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="theme-color" content="#faf8f4">
{FAVICON_LINK}
<title>AI Digest</title>
<script>(function(){{try{{var t=localStorage.getItem('digest-theme');if(t==='light'||t==='dark')document.documentElement.setAttribute('data-theme',t);var dk=t==='dark'||(t!=='light'&&window.matchMedia&&matchMedia('(prefers-color-scheme: dark)').matches);var m=document.querySelector('meta[name="theme-color"]');if(m)m.setAttribute('content',dk?'#141311':'#faf8f4');}}catch(e){{}}}})();</script>
{perf_head}<style>
:root {{
  --font-sans: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI",
    "PingFang SC", "Hiragino Sans GB", "HarmonyOS Sans SC", "Microsoft YaHei",
    "Noto Sans CJK SC", "Noto Sans CJK TC", "Noto Sans", sans-serif;
  --radius-panel: 14px;
  --paper-max: 1100px; /* layout2: index content capped at ~1100px, centered */
  --gutter: max(16px, env(safe-area-inset-left));
  --bg: #faf8f4;
  --bg-sunk: #f2eee7;
  --surface: #ffffff;
  --ink: #1f1c18;
  --ink-2: #363029;
  --ink-3: #635a4f;
  --ink-4: #7b7164;
  --line: #e4ddd2;
  --line-strong: #d4cabd;
  --line-soft: #ede7dd;
  --accent: #b85432;
  --accent-ink: #9c4223;
  --well: #f3efe8;
  --shadow-soft: 0 4px 14px rgba(31, 24, 16, 0.06);
  --btn-glint: rgba(255, 255, 255, 0.55);
  color-scheme: light;
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) {{
    --bg: #141311;
    --bg-sunk: #1c1a17;
    --well: #1b1916;
    --surface: #1f1d19;
    --ink: #efeae2;
    --ink-2: #d8d1c7;
    --ink-3: #a89f92;
    --ink-4: #877f73;
    --line: rgba(239, 234, 226, 0.10);
    --line-strong: rgba(239, 234, 226, 0.17);
    --line-soft: rgba(239, 234, 226, 0.06);
    --accent: #df7d56;
    --accent-ink: #ec9672;
    --shadow-soft: 0 6px 20px rgba(0, 0, 0, 0.42);
    --btn-glint: rgba(255, 255, 255, 0.07);
    color-scheme: dark;
  }}
}}
:root[data-theme="dark"] {{
  --bg: #141311;
  --bg-sunk: #1c1a17;
  --well: #1b1916;
  --surface: #1f1d19;
  --ink: #efeae2;
  --ink-2: #d8d1c7;
  --ink-3: #a89f92;
  --ink-4: #877f73;
  --line: rgba(239, 234, 226, 0.10);
  --line-strong: rgba(239, 234, 226, 0.17);
  --line-soft: rgba(239, 234, 226, 0.06);
  --accent: #df7d56;
  --accent-ink: #ec9672;
  --shadow-soft: 0 6px 20px rgba(0, 0, 0, 0.42);
  --btn-glint: rgba(255, 255, 255, 0.07);
  color-scheme: dark;
}}
* {{ box-sizing: border-box; }}
html {{ -webkit-text-size-adjust: 100%; text-rendering: optimizeLegibility; -webkit-font-smoothing: antialiased; }}
body {{
  margin: 0;
  background: color-mix(in srgb, var(--bg) 60%, var(--surface));
  color: var(--ink);
  font-family: var(--font-sans);
  font-size: 14px;
  line-height: 1.5;
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) body {{ background: var(--bg); }}
}}
:root[data-theme="dark"] body {{ background: var(--bg); }}
a {{ color: inherit; text-decoration: none; }}
button {{ cursor: pointer; font-family: inherit; }}
.num {{ font-variant-numeric: tabular-nums; }}
.paper {{
  width: min(var(--paper-max), calc(100% - 2 * max(var(--gutter), 5vw)));
  margin: 0 auto;
  padding: 20px 0 64px;
}}
@media (min-width: 700px) {{ .paper {{ padding: 36px 0 80px; }} }}
.mast-top {{
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  font-size: 12px;
  color: var(--ink-4);
}}
.mast-top .motto {{ display: none; letter-spacing: 0.3em; }}
@media (min-width: 640px) {{ .mast-top .motto {{ display: inline; }} }}
.mast-row {{
  display: flex; align-items: stretch; justify-content: space-between;
  gap: 20px; padding: 24px 0;
}}
@media (min-width: 880px) {{ .mast-row {{ gap: 40px; padding: 32px 0; }} }}
.nameplate h1 {{
  margin: 0;
  font-size: clamp(36px, 7vw, 72px);
  font-weight: 900;
  line-height: 0.95;
  letter-spacing: -0.04em;
  color: var(--ink);
}}
.nameplate .ai {{ color: var(--accent); }}
.imprint {{
  margin: 12px 0 0;
  font-size: 11.5px;
  letter-spacing: 0.36em;
  color: var(--ink-4);
}}
.mark {{
  display: flex; flex-shrink: 0; align-items: stretch;
  background: var(--well);
  box-shadow: inset 0 0 0 1px var(--line-soft);
  border-radius: var(--radius-panel);
  overflow: hidden;
}}
.mark-date {{
  width: 112px; display: flex; flex-direction: column;
  align-items: center; justify-content: center; text-align: center;
  padding: 12px 8px;
}}
@media (min-width: 880px) {{ .mark-date {{ width: 150px; padding: 16px 8px; }} }}
.mark-kicker {{ font-size: 11px; letter-spacing: 0.2em; color: var(--ink-4); }}
.mark-figure {{
  margin-top: 8px; font-size: 28px; font-weight: 900;
  line-height: 0.95; letter-spacing: -0.04em; color: var(--ink);
  white-space: nowrap;
}}
@media (min-width: 880px) {{ .mark-figure {{ font-size: 36px; }} }}
.mark-top {{ margin-top: 8px; font-size: 11.5px; color: var(--ink-2); }}
.mark-bottom {{ font-size: 11.5px; color: var(--ink-4); }}
.kicker {{
  display: flex; align-items: center; gap: 10px;
  margin: 32px 0 14px;
  font-size: 12px; font-weight: 600; letter-spacing: 0.3em;
  color: var(--accent);
}}
.kicker .bar {{
  display: inline-block; width: 24px; height: 2px;
  border-radius: 999px; background: var(--accent);
}}
.kicker .cnt {{
  margin-left: auto; letter-spacing: 0; font-weight: 500; color: var(--ink-4);
}}
.grp {{ border-top: 1px solid var(--line-strong); }}
.grp:last-of-type {{ border-bottom: 1px solid var(--line-strong); }}
.grp > summary {{
  display: flex; align-items: center; gap: 10px 14px; flex-wrap: wrap;
  padding: 14px 2px; cursor: pointer; list-style: none;
  -webkit-user-select: none; user-select: none;
}}
.grp > summary::-webkit-details-marker {{ display: none; }}
.grp > summary:hover .g-range {{ color: var(--accent-ink); }}
.grp > summary:focus-visible {{ outline: 2px solid var(--accent); outline-offset: 2px; border-radius: 6px; }}
.chev {{
  width: 8px; height: 8px; flex: 0 0 auto; margin: 0 2px 2px 2px;
  border-right: 1.5px solid var(--ink-4); border-bottom: 1.5px solid var(--ink-4);
  transform: rotate(-45deg); transition: transform .15s ease;
}}
.grp[open] > summary .chev {{ transform: rotate(45deg); margin-bottom: 4px; }}
.g-range {{ font-size: 16px; font-weight: 700; color: var(--ink); letter-spacing: 0.02em; }}
.g-meta {{ font-size: 12px; color: var(--ink-4); letter-spacing: 0.06em; }}
.wk-weekly {{
  margin-left: auto; font-size: 12px; font-weight: 600; letter-spacing: 0.08em;
  color: var(--accent-ink); padding: 3px 10px; border-radius: 999px;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 35%, transparent);
}}
.wk-weekly:hover {{ background: color-mix(in srgb, var(--accent) 10%, transparent); }}
.issue-list {{ list-style: none; margin: 0 0 10px; padding: 0 0 0 24px; }}
.issue-list li + li {{ border-top: 1px solid var(--line-soft); }}
.issue-list a {{
  display: flex; align-items: baseline; gap: 6px 16px;
  padding: 12px 2px;
}}
.issue-list a:hover .lab {{ color: var(--accent-ink); }}
.issue-list .when {{
  flex: 0 0 auto; min-width: 3.4em;
  font-size: 13px; font-weight: 600; color: var(--ink-2);
}}
.issue-list.weekly .when {{ min-width: 7.6em; }}
.issue-list .dow {{ flex: 0 0 auto; font-size: 12px; color: var(--ink-4); }}
.issue-list .body {{ flex: 1 1 auto; min-width: 0; display: flex; flex-direction: column; gap: 3px; }}
.issue-list .lab {{ font-size: 15px; font-weight: 600; line-height: 1.45; color: var(--ink); }}
.issue-list .sub {{
  font-size: 12.5px; line-height: 1.5; color: var(--ink-3);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}}
.issue-list .cnt {{ flex: 0 0 auto; font-size: 12px; color: var(--ink-4); }}
@media (max-width: 560px) {{
  .issue-list {{ padding-left: 0; }}
  .issue-list a {{ flex-wrap: wrap; }}
  .issue-list .body {{ flex-basis: 100%; order: 3; }}
  .issue-list .cnt {{ margin-left: auto; }}
  .issue-list .sub {{ white-space: normal; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; }}
}}
.lock {{
  height: 26px; padding: 0 8px; border: 0; border-radius: 999px;
  background: transparent; color: var(--ink-4); font-size: 12px;
}}
.lock:hover {{ color: var(--ink); background: var(--bg-sunk); }}
.lang-link {{ font-size: 12px; color: var(--ink-4); text-decoration: none; padding: 0 8px; margin-right: 4px; }}
.lang-link:hover {{ color: var(--accent-ink); }}
.empty {{ color: var(--ink-4); font-size: 13px; }}
.colophon {{ padding: 48px 0 0; text-align: center; }}
.colophon .end {{
  font-size: 13px; font-weight: 600; letter-spacing: 0.6em; color: var(--ink-4);
}}
/* Compact masthead: full-bleed topbar sits above .paper */
.paper {{ padding-top: 16px; }}
@media (min-width: 700px) {{ .paper {{ padding-top: 22px; }} }}
.mast-bar {{
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 0 9px;
  min-width: 0;
}}
.mast-bar .np {{
  margin: 0;
  font-size: 22px;
  font-weight: 900;
  line-height: 1;
  letter-spacing: -0.03em;
  color: var(--ink);
  white-space: nowrap;
}}
@media (min-width: 880px) {{ .mast-bar .np {{ font-size: 28px; }} }}
.mast-bar .np .ai {{ color: var(--accent); }}
.mast-bar .np a.np-link {{
  color: inherit;
  text-decoration: none;
  display: inline-flex;
  align-items: baseline;
  border-radius: 6px;
  transition: opacity 150ms ease;
}}
.mast-bar .np a.np-link:hover {{ opacity: 0.82; }}
.mast-bar .np a.np-link:focus-visible {{ outline: 2px solid var(--accent); outline-offset: 3px; }}
.mast-chip {{
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  flex-shrink: 0;
  padding: 3px 9px;
  background: var(--well);
  box-shadow: inset 0 0 0 1px var(--line-soft);
  border-radius: 999px;
  white-space: nowrap;
}}
.mast-chip .mark-kicker {{ font-size: 10.5px; letter-spacing: 0.15em; color: var(--ink-4); }}
.mast-chip .mark-figure {{
  margin: 0;
  font-size: 15px;
  font-weight: 900;
  line-height: 1;
  letter-spacing: -0.02em;
  color: var(--ink);
}}
.mast-meta {{
  display: none;
  min-width: 0;
  gap: 14px;
  font-size: 12px;
  color: var(--ink-4);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}}
.mast-meta .motto {{ display: none; letter-spacing: 0.3em; }}
html[lang="en"] .mast-meta .motto, html[lang="en"] .kicker, html[lang="en"] .mark-kicker {{ letter-spacing: 0.06em; }}
html[lang="en"] .kicker, html[lang="en"] .mark-kicker {{ text-transform: uppercase; }}
@media (min-width: 640px) {{ .mast-meta {{ display: inline-flex; }} }}
@media (min-width: 1000px) {{ .mast-meta .motto {{ display: inline; }} }}
.mast-bar .mast-tools {{ display: inline-flex; align-items: center; flex-shrink: 0; margin-left: auto; font-size: 12px; color: var(--ink-4); gap: 8px; }}
@media (max-width: 639px) {{
  .mast-bar {{ gap: 8px; }}
  .mast-bar .edition {{ display: none; }}
}}
.search-btn .search-kbd {{
  margin-left: 1px; padding: 0 4px; border: 1px solid var(--line); border-radius: 4px;
  background: var(--bg-sunk); color: var(--ink-4); font-size: 10px; line-height: 1.45;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}}
.search-btn.is-open, .search-btn.has-filter {{
  border-color: var(--accent); color: var(--accent-ink);
  background: color-mix(in srgb, var(--accent) 10%, var(--surface));
}}
.search-btn.is-open .search-kbd, .search-btn.has-filter .search-kbd {{ display: none; }}
.search-badge {{
  padding: 0 5px; border-radius: 999px; background: var(--accent); color: #fff;
  font-size: 10px; font-weight: 600; line-height: 1.55;
}}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) .search-badge {{ color: #161310; }} }}
:root[data-theme="dark"] .search-badge {{ color: #161310; }}
@media (hover: none), (max-width: 639px) {{ .search-btn .search-kbd, .search-btn .ctrl-lbl {{ display: none; }} }}
.filter-bar {{
  margin: 0; padding: 9px 0 11px;
  border-top: 1px solid var(--line-soft);
  background: transparent;
  display: flex; flex-direction: column; gap: 7px;
}}
.filter-bar[hidden] {{ display: none !important; }}
.filter-bar > .filter-row, .filter-bar > .filter-tags {{
  width: min(var(--paper-max), calc(100% - 2 * max(var(--gutter), 5vw)));
  margin-inline: auto;
}}
@media (max-width: 639px) {{ .filter-bar {{ padding: 8px 0 10px; }} }}
.filter-row {{ display: flex; align-items: center; gap: 7px; position: relative; }}
.filter-input {{
  flex: 1 1 auto; min-width: 0; height: 33px; padding: 0 11px;
  border: 1px solid var(--line); border-radius: 9px; background: var(--surface); color: var(--ink);
  font-family: inherit; font-size: 13.5px; outline: none;
  transition: border-color 150ms ease, box-shadow 180ms ease;
}}
.filter-input::placeholder {{ color: var(--ink-4); }}
.filter-input:focus {{
  border-color: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent);
}}
.filter-count {{ flex: 0 0 auto; font-size: 12px; color: var(--ink-4); padding: 0 3px; white-space: nowrap; }}
.filter-clear, .filter-close {{
  flex: 0 0 auto; height: 26px; padding: 0 10px; border: 1px solid var(--line);
  border-radius: 999px; background: var(--surface); color: var(--ink-3); font-size: 12px;
  transition: transform 240ms cubic-bezier(0.34, 1.56, 0.64, 1), color 140ms ease, border-color 140ms ease;
}}
.filter-close {{ width: 26px; padding: 0; display: inline-flex; align-items: center; justify-content: center; font-size: 15px; line-height: 1; }}
.filter-clear:hover, .filter-close:hover {{ color: var(--ink); border-color: var(--accent); transform: translateY(-1px); }}
.filter-clear:active, .filter-close:active {{ transform: scale(0.93); }}
.filter-tags {{
  display: flex; flex-wrap: nowrap; align-items: center; gap: 5px;
  overflow-x: auto; -webkit-overflow-scrolling: touch; scrollbar-width: none; padding: 1px 18px 2px 0;
  mask-image: linear-gradient(90deg, #000 94%, transparent);
  -webkit-mask-image: linear-gradient(90deg, #000 94%, transparent);
}}
.filter-tags::-webkit-scrollbar {{ display: none; }}
.filter-tag {{
  flex: 0 0 auto; height: 25px; padding: 0 10px; border: 1px solid var(--line);
  border-radius: 999px; background: var(--surface); color: var(--ink-3); font-size: 11.5px; white-space: nowrap;
  display: inline-flex; align-items: center; gap: 4px;
  transition: transform 240ms cubic-bezier(0.34, 1.56, 0.64, 1), color 140ms ease, border-color 140ms ease, background-color 140ms ease;
}}
.filter-tag .ft-cnt {{ font-size: 10.5px; opacity: 0.6; }}
.filter-tag:hover {{ color: var(--accent-ink); border-color: var(--accent); transform: translateY(-1px); }}
.filter-tag:active {{ transform: scale(0.93); }}
.filter-tag[aria-pressed="true"] {{
  background: color-mix(in srgb, var(--accent) 14%, var(--surface));
  border-color: var(--accent);
  color: var(--accent-ink);
  font-weight: 600;
}}
.filter-tag[aria-pressed="true"] .ft-cnt {{ opacity: 0.85; }}
.filter-empty {{ margin: 20px 0 0; color: var(--ink-4); font-size: 13.5px; text-align: center; }}

/* Sticky auto-hide TopBar + physics-spring buttons + latest quick pills */
@keyframes ctrlPop {{ 0% {{ transform: scale(0.92); }} 55% {{ transform: scale(1.045); }} 100% {{ transform: scale(1); }} }}
.mast {{
  position: sticky; top: 0; z-index: 30;
  width: 100%;
  background: color-mix(in srgb, var(--surface) 88%, transparent);
  -webkit-backdrop-filter: blur(16px) saturate(160%);
  backdrop-filter: blur(16px) saturate(160%);
  border-bottom: 1px solid var(--line-soft);
  transition: transform 280ms cubic-bezier(0.22, 1, 0.36, 1), background-color 180ms ease, box-shadow 200ms ease, border-color 200ms ease;
  will-change: transform;
}}
.mast > .mast-bar {{
  width: min(var(--paper-max), calc(100% - 2 * max(var(--gutter), 5vw)));
  margin-inline: auto;
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) .mast {{
    background: color-mix(in srgb, var(--bg-sunk) 88%, transparent);
  }}
}}
:root[data-theme="dark"] .mast {{
  background: color-mix(in srgb, var(--bg-sunk) 88%, transparent);
}}
.mast.is-scrolled {{
  background: color-mix(in srgb, var(--surface) 92%, transparent);
  border-bottom-color: var(--line);
  box-shadow: 0 2px 14px rgba(31, 24, 16, 0.08);
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) .mast.is-scrolled {{
    background: color-mix(in srgb, var(--bg-sunk) 92%, transparent);
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.42);
  }}
}}
:root[data-theme="dark"] .mast.is-scrolled {{
  background: color-mix(in srgb, var(--bg-sunk) 92%, transparent);
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.42);
}}
.mast.is-hidden {{ transform: translateY(calc(-100% - 2px)); box-shadow: none !important; pointer-events: none; }}
.ctrl-btn {{
  position: relative; overflow: hidden; display: inline-flex; align-items: center; justify-content: center; gap: 5px;
  height: 30px; padding: 0 11px; border: 1px solid var(--line); border-radius: 999px;
  background: color-mix(in srgb, var(--surface) 88%, var(--well)); color: var(--ink-2);
  font-size: 12px; font-weight: 560; white-space: nowrap; text-decoration: none; user-select: none;
  box-shadow: inset 0 1px 0 var(--btn-glint), 0 1px 2px rgba(31, 24, 16, 0.05);
  transition: transform 280ms cubic-bezier(0.34, 1.56, 0.64, 1), border-color 160ms ease, background-color 160ms ease, color 160ms ease, box-shadow 200ms ease;
}}
.ctrl-btn::after {{
  content: ""; position: absolute; inset: 0;
  background: linear-gradient(115deg, transparent 22%, rgba(255, 255, 255, 0.28) 50%, transparent 78%);
  transform: translateX(-125%); transition: transform 520ms cubic-bezier(0.22, 1, 0.36, 1); pointer-events: none;
}}
.ctrl-btn:hover {{
  color: var(--accent-ink); border-color: color-mix(in srgb, var(--accent) 48%, var(--line));
  background: color-mix(in srgb, var(--accent) 8%, var(--surface)); transform: translateY(-1px);
  box-shadow: inset 0 1px 0 var(--btn-glint), 0 4px 12px color-mix(in srgb, var(--accent) 18%, transparent);
}}
.ctrl-btn:hover::after {{ transform: translateX(125%); }}
.ctrl-btn:active {{ transform: translateY(0) scale(0.93); transition-duration: 90ms; }}
.ctrl-btn.is-pop {{ animation: ctrlPop 340ms cubic-bezier(0.34, 1.56, 0.64, 1); }}
.ctrl-ic {{ display: inline-flex; align-items: center; justify-content: center; width: 14px; height: 14px; flex-shrink: 0; color: var(--accent); transition: transform 380ms cubic-bezier(0.34, 1.56, 0.64, 1); }}
.lang-btn:hover .ctrl-ic {{ transform: rotate(28deg); }}
.theme-ics {{ position: relative; display: inline-flex; align-items: center; justify-content: center; width: 14px; height: 14px; color: var(--accent); }}
.theme-ics svg {{ position: absolute; inset: 0; opacity: 0; transform: scale(0.4) rotate(-65deg); transition: transform 360ms cubic-bezier(0.34, 1.56, 0.64, 1), opacity 180ms ease; }}
.theme-btn[data-mode="system"] .ic-sys, .theme-btn[data-mode="light"] .ic-sun, .theme-btn[data-mode="dark"] .ic-moon {{ opacity: 1; transform: scale(1) rotate(0deg); }}
@media (max-width: 479px) {{
  .ctrl-btn {{ height: 28px; padding: 0 9px; font-size: 11.5px; }}
  .theme-btn .ctrl-lbl {{ display: none; }}
  .theme-btn {{ width: 30px; padding: 0; }}
}}
.quick-bar {{ display: flex; flex-wrap: wrap; gap: 8px; margin: 14px 0 4px; }}
.quick-pill {{
  display: inline-flex; align-items: center; gap: 7px; height: 32px; padding: 0 13px;
  border: 1px solid var(--line); border-radius: 999px; background: var(--surface); color: var(--ink);
  font-size: 12.5px; font-weight: 600; text-decoration: none;
  box-shadow: 0 1px 2px rgba(31, 24, 16, 0.04);
  transition: transform 260ms cubic-bezier(0.34, 1.56, 0.64, 1), border-color 150ms ease, color 150ms ease, box-shadow 180ms ease;
}}
.quick-pill:hover {{
  border-color: var(--accent); color: var(--accent-ink); transform: translateY(-1px);
  box-shadow: 0 4px 12px color-mix(in srgb, var(--accent) 16%, transparent);
}}
.quick-pill:active {{ transform: scale(0.95); transition-duration: 80ms; }}
.qp-dot {{ width: 6px; height: 6px; border-radius: 50%; background: var(--accent); }}
.qp-meta {{ font-size: 11.5px; font-weight: 500; color: var(--ink-4); }}
.issue-list li {{ content-visibility: auto; contain-intrinsic-size: auto 56px; }}
@media (prefers-reduced-motion: reduce) {{
  *, *::before, *::after {{ transition: none !important; animation: none !important; }}
}}
@media print {{ .mast {{ position: static; margin: 0; padding: 0; background: none; backdrop-filter: none; box-shadow: none; transform: none !important; }} .ctrl-btn, .theme-switch, .lang-switch, .quick-bar, .filter-bar, .filter-empty {{ display: none !important; }} .issue-list li {{ content-visibility: visible; }} }}
</style>
</head>
<body>
<header class="mast">
<div class="mast-bar">
<h1 class="np"><a class="np-link" href="./"><span class="ai">AI</span> Digest</a></h1>
<span class="mast-chip"><span class="mark-kicker">{_esc(tr("index_chip"))}</span><span class="mark-figure num">{len(issues)}</span><span class="mark-kicker">{_esc(i18n.issue_unit(lg, len(issues)))}</span></span>
<span class="mast-meta"><span class="num">{_esc(today_line)}</span><span>{_esc(tr("index_meta", d=len(dailies), w=len(weeklies)))}</span><span class="motto">{_esc(tr("motto"))}</span></span>
<span class="mast-tools">
{search_toggle_btn}
{lang_btn}
{theme_toggle_btn}
{lock_btn}
</span>
</div>
{filter_drawer}
</header>
<article class="paper">
{quick_bar}
{filter_empty}
<section class="archive-sec">
<div class="kicker"><span class="bar"></span>{_esc(tr("index_daily"))}<span class="cnt num">{_esc(i18n.count_issues(lg, len(dailies)))}</span></div>
{daily_ol}
</section>
<section class="archive-sec">
<div class="kicker"><span class="bar"></span>{_esc(tr("index_weekly"))}<span class="cnt num">{_esc(i18n.count_issues(lg, len(weeklies)))}</span></div>
{weekly_ol}
</section>
<footer class="colophon"><div class="end">{_esc(tr("index_end"))}</div></footer>
</article>
{lock_js}
<script>
(function(){{
  const KEY='digest-theme';
  const MODES=['system','light','dark'];
  const root=document.documentElement;
  const tbtn=document.querySelector('[data-theme-toggle]');
  const meta=document.querySelector('meta[name="theme-color"]');
  const mq=window.matchMedia?matchMedia('(prefers-color-scheme: dark)'):null;
  let curTheme='system';
  function pop(el){{if(!el)return;el.classList.remove('is-pop');void el.offsetWidth;el.classList.add('is-pop');}}
  function paintMeta(){{
    const isDark=root.getAttribute('data-theme')==='dark'||(!root.getAttribute('data-theme')&&Boolean(mq&&mq.matches));
    if(meta)meta.setAttribute('content',isDark?'#141311':'#faf8f4');
  }}
  function apply(v,anim){{
    curTheme=MODES.includes(v)?v:'system';
    if(curTheme==='light'||curTheme==='dark') root.setAttribute('data-theme', curTheme);
    else root.removeAttribute('data-theme');
    if(tbtn){{
      tbtn.dataset.mode=curTheme;
      let lbls={{}};try{{lbls=JSON.parse(tbtn.dataset.labels||'{{}}');}}catch(e){{}}
      const txt=lbls[curTheme]||curTheme;
      const sp=tbtn.querySelector('.ctrl-lbl');
      if(sp)sp.textContent=txt;
      tbtn.setAttribute('aria-label',(tbtn.dataset.ariaBase||'Appearance')+': '+txt);
      if(anim)pop(tbtn);
    }}
    paintMeta();
  }}
  try{{curTheme=localStorage.getItem(KEY)||'system';}}catch(e){{}}
  apply(curTheme,false);
  try{{if(mq&&mq.addEventListener)mq.addEventListener('change',paintMeta);}}catch(e){{}}
  function cycleTheme(){{
    const nxt=MODES[(MODES.indexOf(curTheme)+1)%MODES.length];
    try{{localStorage.setItem(KEY,nxt);}}catch(e){{}}
    apply(nxt,true);
  }}
  if(tbtn)tbtn.addEventListener('click',cycleTheme);

  const mast=document.querySelector('header.mast');
  const bar=document.getElementById('archiveFilterBar');
  const stgl=document.querySelector('[data-search-toggle]');
  const sbadge=stgl?stgl.querySelector('[data-search-badge]'):null;
  let showMast=function(){{}},searchOpen=Boolean(bar&&!bar.hidden);
  if(mast){{
    let lastY=Math.max(0,scrollY),sRaf=0,stopTimer=0,accDown=0,accUp=0,mastHover=false,mastFocus=false,barHidden=false;
    function setHide(h){{
      const nh=Boolean(h&&!(mastHover||mastFocus||searchOpen)&&scrollY>64);
      barHidden=nh;
      mast.classList.toggle('is-hidden',nh);
      if(!nh){{clearTimeout(stopTimer);stopTimer=0;}}
    }}
    showMast=()=>setHide(false);
    function armIdle(){{
      clearTimeout(stopTimer);
      stopTimer=setTimeout(()=>{{stopTimer=0;if(barHidden)setHide(false);}},3000);
    }}
    mast.addEventListener('pointerenter',e=>{{if(e.pointerType==='mouse'){{mastHover=true;setHide(false);}}}});
    mast.addEventListener('pointerleave',e=>{{if(e.pointerType==='mouse'){{mastHover=false;}}}});
    mast.addEventListener('focusin',e=>{{let kv=false;try{{kv=Boolean(e.target&&e.target.matches&&e.target.matches(':focus-visible'));}}catch(_){{}}if(kv){{mastFocus=true;setHide(false);}}}});
    mast.addEventListener('focusout',e=>{{if(!mast.contains(e.relatedTarget))mastFocus=false;}});
    function onFrame(){{
      sRaf=0;
      const y=Math.max(0,scrollY),dy=y-lastY;lastY=y;
      mast.classList.toggle('is-scrolled',y>20);
      if(y<=64){{accDown=0;accUp=0;setHide(false);return;}}
      if(dy>0){{
        accDown+=dy;accUp=0;
        if(accDown>=14){{setHide(true);accDown=0;}}
        if(barHidden)armIdle();
      }}else if(dy<0){{
        accUp+=-dy;accDown=0;
        if(accUp>=12){{setHide(false);accUp=0;}}
        else if(barHidden)armIdle();
      }}else if(barHidden){{
        armIdle();
      }}
    }}
    addEventListener('scroll',()=>{{if(!sRaf)sRaf=requestAnimationFrame(onFrame);}},{{passive:true}});
    addEventListener('pointermove',e=>{{if(e.clientY<=12)setHide(false);}},{{passive:true}});
  }}

  let openSearch=function(){{}};
  if(bar){{
    const inp=document.getElementById('archiveFilterInput');
    const clr=document.getElementById('archiveFilterClear');
    const cls=bar.querySelector('.filter-close');
    const cnt=document.getElementById('archiveFilterCount');
    const emp=document.getElementById('archiveFilterEmpty');
    const tbtns=[...bar.querySelectorAll('.filter-tag')];
    const items=[...document.querySelectorAll('li.archive-item')].map(el=>({{
      el:el,hay:el.dataset.search||'',tgs:(el.dataset.tags||'').split('|').filter(Boolean)
    }}));
    const grpGroups=[...document.querySelectorAll('details.grp')].map(g=>({{
      g:g,initOpen:g.open,list:[...g.querySelectorAll('li.archive-item')]
    }}));
    const secGroups=[...document.querySelectorAll('section.archive-sec')].map(s=>({{
      s:s,gs:[...s.querySelectorAll('details.grp')]
    }}));
    const tpl=bar.dataset.tpl||'{{shown}} / {{total}}';
    let curTag='',fRaf=0;
    function setSearchOpen(open,focusInput){{
      searchOpen=Boolean(open);
      bar.hidden=!searchOpen;
      if(stgl){{
        stgl.setAttribute('aria-expanded',searchOpen?'true':'false');
        stgl.classList.toggle('is-open',searchOpen);
      }}
      if(searchOpen)showMast();
      if(searchOpen&&focusInput&&inp){{inp.focus();inp.select();}}
    }}
    openSearch=()=>setSearchOpen(true,true);
    function resetFilter(){{inp.value='';curTag='';tbtns.forEach(b=>b.setAttribute('aria-pressed',b.dataset.tag===''?'true':'false'));runFilter();}}
    function runFilter(){{
      fRaf=0;
      const q=(inp.value||'').trim().toLowerCase();
      const toks=q?q.split(/\\s+/):[];
      const active=Boolean(toks.length||curTag);
      let shown=0;
      for(let i=0;i<items.length;i++){{
        const it=items[i];
        const ok=(!curTag||it.tgs.includes(curTag))&&toks.every(k=>it.hay.includes(k));
        it.el.hidden=!ok;
        if(ok)shown++;
      }}
      for(let i=0;i<grpGroups.length;i++){{
        const gg=grpGroups[i];
        let has=false;
        for(let j=0;j<gg.list.length;j++){{if(!gg.list[j].hidden){{has=true;break;}}}}
        if(active){{gg.g.hidden=!has;if(has)gg.g.open=true;}}
        else{{gg.g.hidden=false;gg.g.open=gg.initOpen;}}
      }}
      for(let i=0;i<secGroups.length;i++){{
        const sg=secGroups[i];
        if(sg.gs.length)sg.s.hidden=active&&sg.gs.every(g=>g.hidden);
      }}
      bar.classList.toggle('is-active',active);
      clr.hidden=!active;
      cnt.hidden=!active;
      if(active)cnt.textContent=tpl.replace('{{shown}}',String(shown)).replace('{{total}}',String(items.length));
      if(stgl)stgl.classList.toggle('has-filter',active);
      if(sbadge){{sbadge.hidden=!active;if(active)sbadge.textContent=shown+'/'+items.length;}}
      if(emp)emp.hidden=!(active&&shown===0);
    }}
    function sched(){{if(!fRaf)fRaf=requestAnimationFrame(runFilter);}}
    if(stgl)stgl.addEventListener('click',()=>{{pop(stgl);setSearchOpen(!searchOpen,!searchOpen);}});
    if(cls)cls.addEventListener('click',()=>{{setSearchOpen(false,false);if(stgl)stgl.focus();}});
    inp.addEventListener('input',sched);
    clr.addEventListener('click',()=>{{resetFilter();inp.focus();}});
    tbtns.forEach(b=>b.addEventListener('click',()=>{{
      const tg=b.dataset.tag||'';
      curTag=(tg&&curTag===tg)?'':tg;
      tbtns.forEach(x=>x.setAttribute('aria-pressed',(x.dataset.tag||'')===curTag?'true':'false'));
      runFilter();
    }}));
    addEventListener('keydown',e=>{{
      if(e.key==='Escape'&&searchOpen){{
        e.preventDefault();
        if(inp.value||curTag)resetFilter();
        else{{setSearchOpen(false,false);if(stgl)stgl.focus();}}
      }}
    }});
  }}
  addEventListener('keydown',e=>{{
    if(e.metaKey||e.ctrlKey||e.altKey)return;
    if(/^(INPUT|TEXTAREA|SELECT)$/.test((e.target&&e.target.tagName)||''))return;
    if(e.key==='/'){{if(bar){{e.preventDefault();openSearch();}}}}
    else if(e.key==='t'||e.key==='T'){{e.preventDefault();cycleTheme();showMast();}}
    else if(e.key==='l'||e.key==='L'){{const a=document.querySelector('a.lang-btn[href]');if(a&&a.href){{e.preventDefault();location.href=a.href;}}}}
  }});
  if(location.hash.length>1){{
    let id=location.hash.slice(1);try{{id=decodeURIComponent(id);}}catch(e){{}}
    const el=document.getElementById(id);
    if(el)requestAnimationFrame(()=>el.scrollIntoView());
  }}
}})();
</script>
</body>
</html>
"""


def _esc(s: str) -> str:
    return htmlmod.escape(s, quote=False).replace('"', "&quot;")


def _clear_files(root: Path) -> None:
    if not root.exists():
        return
    for p in root.rglob("*"):
        if p.is_file():
            p.unlink()


def _clear_root_latest(base: Path) -> None:
    """Remove any generated root-level latest alias files (and empty daily/weekly/zh/en dirs)."""
    for rel in _ROOT_LATEST_RELS:
        p = base / rel
        if p.is_file():
            p.unlink()
    for sub in ("zh/daily", "zh/weekly", "zh", "en/daily", "en/weekly", "en", "daily", "weekly"):
        d = base / sub
        if d.is_dir():
            try:
                d.rmdir()
            except OSError:
                pass


def _render_issue_page(
    issue: Issue,
    issues: list[Issue],
    *,
    slug: str,
    alias_rel: str | None = None,
    lang: str | None = None,
    url_prefix: str | None = None,
    lang_href: str | None = None,
    lang_code: str | None = None,
    ctx: SiteContext | None = None,
) -> str:
    abs_links = alias_rel is not None
    html = issue.source.read_text(encoding="utf-8")
    html = strip_injections(html)
    html = ensure_noindex(html)
    html = ensure_favicon(html)
    html = ensure_nav_css(html)
    html = inject_nav(
        html,
        issue,
        issues,
        lang=lang,
        slug=slug,
        abs_links=abs_links,
        lang_rel=alias_rel,
        url_prefix=url_prefix,
        lang_href=lang_href,
        lang_code=lang_code,
        ctx=ctx,
    )
    html = inject_perf_hints(
        html, issue, issues, slug=slug, abs_links=abs_links, url_prefix=url_prefix, ctx=ctx
    )
    return minify_css(html)


def rebuild_all(cfg: dict[str, str], ctx: SiteContext | None = None) -> list[Issue]:
    """Build pure plaintext (nav already injected, no encryption/lock logic) under site/plain/.

    Does not write issue HTML into public/; publish_plain_pages or site_crypto.encrypt_all_pages does that.
    Root files (robots.txt, 404, base _headers, _redirects) are written unencrypted to public/.
    """
    c = ctx or _default_ctx()
    slug = cfg["SLUG"]
    root_latest = _parse_bool(cfg.get("ROOT_LATEST"))
    issues = list_sources(ctx=c)
    en_issues = list_en_sources(ctx=c)
    has_lang_paths = bool(en_issues) and c.lang == "zh-CN"
    write_static_root(
        slug=slug,
        issues=issues,
        root_latest=root_latest,
        has_lang_paths=has_lang_paths,
        ctx=c,
    )
    plain_dir = _site_dir(c.plain, slug)
    _clear_files(plain_dir)
    if slug and not c.demo_mode:
        _clear_root_latest(c.plain)
    (plain_dir / "daily").mkdir(parents=True, exist_ok=True)
    (plain_dir / "weekly").mkdir(parents=True, exist_ok=True)

    en_keys = {(i.kind, i.label) for i in en_issues}
    zh_keys = {(i.kind, i.label) for i in issues}
    en_daily, en_weekly, en_any = _latest_issues(en_issues)
    zh_daily, zh_weekly, zh_any = _latest_issues(issues)
    zh_base = _url_prefix(slug, "zh")
    en_base = _url_prefix(slug, "en")

    def _en_target_for(kind: str, label: str) -> str:
        return f"{en_base}/{kind}/{label}" if (kind, label) in en_keys else f"{en_base}/"

    def _en_latest_target(alias_rel: str, *, root_level: bool) -> str:
        base_en = "/en" if root_level else en_base
        if alias_rel == "daily/latest":
            return f"{base_en}/daily/latest" if en_daily else (f"{base_en}/latest" if root_level else f"{base_en}/")
        if alias_rel == "weekly/latest":
            return f"{base_en}/weekly/latest" if en_weekly else (f"{base_en}/latest" if root_level else f"{base_en}/")
        return f"{base_en}/latest" if en_any else (f"{base_en}/latest" if root_level else f"{base_en}/")

    def _zh_latest_target(alias_rel: str, *, root_level: bool) -> str:
        base_zh = "/zh" if root_level else zh_base
        if alias_rel == "daily/latest":
            return f"{base_zh}/daily/latest" if zh_daily else (f"{base_zh}/latest" if root_level else f"{base_zh}/")
        if alias_rel == "weekly/latest":
            return f"{base_zh}/weekly/latest" if zh_weekly else (f"{base_zh}/latest" if root_level else f"{base_zh}/")
        return f"{base_zh}/latest" if zh_any else (f"{base_zh}/latest" if root_level else f"{base_zh}/")

    # 1. Legacy unprefixed paths under plain_dir (root when SLUG='', or /{slug}/ when SLUG is set)
    for issue in issues:
        leg_href = None if c.lang_alt else (_en_target_for(issue.kind, issue.label) if has_lang_paths else None)
        leg_code = "en" if leg_href else None
        html = _render_issue_page(
            issue, issues, slug=slug, lang_href=leg_href, lang_code=leg_code, ctx=c
        )
        dest = plain_dir / issue.kind / f"{issue.label}.html"
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(html, encoding="utf-8")

    for rel_path, issue, lang_rel, is_root in _latest_alias_specs(
        slug, issues, include_root=root_latest and bool(slug)
    ):
        dest = c.plain / rel_path
        dest.parent.mkdir(parents=True, exist_ok=True)
        if is_root and has_lang_paths:
            lh = _en_latest_target(lang_rel, root_level=True)
            html = _render_issue_page(
                issue, issues, slug=slug, alias_rel=lang_rel,
                url_prefix=zh_base, lang_href=lh, lang_code="en", ctx=c,
            )
        elif not is_root and has_lang_paths and not c.lang_alt:
            lh = _en_latest_target(lang_rel, root_level=False)
            html = _render_issue_page(
                issue, issues, slug=slug, alias_rel=lang_rel,
                lang_href=lh, lang_code="en", ctx=c,
            )
        else:
            html = _render_issue_page(
                issue, issues, slug=slug, alias_rel=lang_rel, ctx=c
            )
        dest.write_text(html, encoding="utf-8")

    leg_idx_href = None if c.lang_alt else (f"{en_base}/" if has_lang_paths else None)
    leg_idx_code = "en" if leg_idx_href else None
    (plain_dir / "index.html").write_text(
        minify_css(index_html(cfg, issues, lang_href=leg_idx_href, lang_code=leg_idx_code, ctx=c)),
        encoding="utf-8",
    )

    # 2. Symmetric /zh/ and /en/ URL trees whenever companion English sources exist
    if has_lang_paths:
        zh_dir = plain_dir / "zh"
        en_dir = plain_dir / "en"
        for d in (zh_dir / "daily", zh_dir / "weekly", en_dir / "daily", en_dir / "weekly"):
            d.mkdir(parents=True, exist_ok=True)

        for issue in issues:
            html = _render_issue_page(
                issue, issues, slug=slug, lang="zh-CN",
                lang_href=_en_target_for(issue.kind, issue.label), lang_code="en", ctx=c,
            )
            (zh_dir / issue.kind / f"{issue.label}.html").write_text(html, encoding="utf-8")
        (zh_dir / "index.html").write_text(
            minify_css(index_html(cfg, issues, lang="zh-CN", lang_href=f"{en_base}/", lang_code="en", ctx=c)),
            encoding="utf-8",
        )

        for alias_rel, issue in (("daily/latest", zh_daily), ("weekly/latest", zh_weekly), ("latest", zh_any)):
            if not issue:
                continue
            html_slug = _render_issue_page(
                issue, issues, slug=slug, alias_rel=alias_rel, lang="zh-CN",
                url_prefix=zh_base, lang_href=_en_latest_target(alias_rel, root_level=False), lang_code="en", ctx=c,
            )
            p_slug = zh_dir / f"{alias_rel}.html"
            p_slug.parent.mkdir(parents=True, exist_ok=True)
            p_slug.write_text(html_slug, encoding="utf-8")
            if root_latest and slug:
                html_root = _render_issue_page(
                    issue, issues, slug=slug, alias_rel=alias_rel, lang="zh-CN",
                    url_prefix=zh_base, lang_href=_en_latest_target(alias_rel, root_level=True), lang_code="en", ctx=c,
                )
                p_root = c.plain / "zh" / f"{alias_rel}.html"
                p_root.parent.mkdir(parents=True, exist_ok=True)
                p_root.write_text(html_root, encoding="utf-8")

        for issue in en_issues:
            zh_href = f"{zh_base}/{issue.kind}/{issue.label}" if (issue.kind, issue.label) in zh_keys else f"{zh_base}/"
            html = _render_issue_page(
                issue, en_issues, slug=slug, lang="en",
                lang_href=zh_href, lang_code="zh-CN", ctx=c,
            )
            (en_dir / issue.kind / f"{issue.label}.html").write_text(html, encoding="utf-8")
        (en_dir / "index.html").write_text(
            minify_css(index_html(cfg, en_issues, lang="en", lang_href=f"{zh_base}/", lang_code="zh-CN", ctx=c)),
            encoding="utf-8",
        )

        for alias_rel, issue in (("daily/latest", en_daily), ("weekly/latest", en_weekly), ("latest", en_any)):
            if not issue:
                continue
            html_slug = _render_issue_page(
                issue, en_issues, slug=slug, alias_rel=alias_rel, lang="en",
                url_prefix=en_base, lang_href=_zh_latest_target(alias_rel, root_level=False), lang_code="zh-CN", ctx=c,
            )
            p_slug = en_dir / f"{alias_rel}.html"
            p_slug.parent.mkdir(parents=True, exist_ok=True)
            p_slug.write_text(html_slug, encoding="utf-8")
            if root_latest and slug:
                html_root = _render_issue_page(
                    issue, en_issues, slug=slug, alias_rel=alias_rel, lang="en",
                    url_prefix=en_base, lang_href=_zh_latest_target(alias_rel, root_level=True), lang_code="zh-CN", ctx=c,
                )
                p_root = c.plain / "en" / f"{alias_rel}.html"
                p_root.parent.mkdir(parents=True, exist_ok=True)
                p_root.write_text(html_root, encoding="utf-8")

    return issues


def privacy_scan(paths: Iterable[Path], *, ctx: SiteContext | None = None) -> None:
    c = ctx or _default_ctx()
    site_crypto.privacy_scan(paths, site_dir=c.site)


def iter_public_files(ctx: SiteContext | None = None) -> list[Path]:
    c = ctx or _default_ctx()
    return site_crypto._iter_files(c.public)


def iter_plain_files(ctx: SiteContext | None = None) -> list[Path]:
    c = ctx or _default_ctx()
    return site_crypto._iter_files(c.plain)


def iter_unencrypted_public(ctx: SiteContext | None = None) -> list[Path]:
    """Root files that stay in plaintext: robots.txt, 404.html, favicon.svg, _headers, _redirects."""
    c = ctx or _default_ctx()
    return site_crypto.iter_unencrypted_public(c.public)


def pages_hash(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()[:32]


def content_type_for(path: Path, rel: str) -> str:
    name = path.name
    if name == "_headers" or name == "_redirects":
        return "text/plain; charset=utf-8"
    if name.endswith(".html"):
        return "text/html; charset=utf-8"
    if name.endswith(".svg"):
        return "image/svg+xml"
    if name.endswith(".txt"):
        return "text/plain; charset=utf-8"
    guessed, _ = mimetypes.guess_type(str(path))
    return guessed or "application/octet-stream"


_PAGES_CONTROL_FILES = frozenset({"_headers", "_redirects", "_worker.js", "_routes.json"})


def build_manifest() -> tuple[dict[str, str], dict[str, dict]]:
    """Return (manifest path->hash, files path->{hash,content,contentType}).

    Excludes Cloudflare Pages control files (_headers, _redirects, _worker.js,
    _routes.json) from the asset manifest — they are sent as dedicated multipart
    form fields so Pages never serves them as static files (which would expose SLUG).
    """
    manifest: dict[str, str] = {}
    files: dict[str, dict] = {}
    for path in iter_public_files():
        rel = path.relative_to(PUBLIC).as_posix()
        if rel in _PAGES_CONTROL_FILES:
            continue
        key = "/" + rel
        data = path.read_bytes()
        h = pages_hash(data)
        manifest[key] = h
        files[key] = {
            "hash": h,
            "contentType": content_type_for(path, rel),
            "base64": base64.b64encode(data).decode("ascii"),
            "size": len(data),
        }
    return manifest, files


def find_token() -> str | None:
    env = os.environ.get("CLOUDFLARE_API_TOKEN", "").strip()
    if env:
        return env
    path = os.environ.get("CLOUDFLARE_API_TOKEN_FILE", "").strip()
    candidates = [Path(path)] if path else []
    candidates.append(TOKEN_FILE_DEFAULT)
    for p in candidates:
        if p and p.is_file():
            mode = p.stat().st_mode
            if mode & (stat.S_IRGRP | stat.S_IROTH):
                print(
                    f"warning: {p} is readable by group/other; chmod 600 recommended",
                    file=sys.stderr,
                )
            tok = p.read_text(encoding="utf-8").strip()
            if tok:
                return tok
    return None


def cf_request(
    token: str,
    method: str,
    path: str,
    body=None,
    content_type: str | None = "application/json",
    form: "bytes | None" = None,
    form_content_type: str | None = None,
) -> dict:
    url = CF_API + path
    data = None
    headers = {
        "Authorization": f"Bearer {token}",
        "User-Agent": "ai-digest-publish/1.0",
    }
    if form is not None:
        data = form
        headers["Content-Type"] = form_content_type or "multipart/form-data"
    elif body is not None:
        data = json.dumps(body).encode("utf-8")
        if content_type:
            headers["Content-Type"] = content_type
    req = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            raw = resp.read()
            return json.loads(raw.decode("utf-8") or "{}")
    except urllib.error.HTTPError as e:
        err = e.read().decode("utf-8", "replace")
        raise RuntimeError(f"{method} {path} -> {e.code}: {err[:2000]}") from None


def deploy_with_wrangler(cfg: dict[str, str], token: str, commit_message: str) -> dict:
    """Deploy public/ with the official Wrangler Pages Direct Upload path."""
    env = os.environ.copy()
    env["CLOUDFLARE_API_TOKEN"] = token
    env["CLOUDFLARE_ACCOUNT_ID"] = cfg["ACCOUNT_ID"]
    env["WRANGLER_SEND_METRICS"] = "false"
    env["CI"] = "true"
    # Keep wrangler's cache off the root-owned /node_modules (find-up would
    # otherwise try to write /node_modules/.cache/wrangler and exit 1
    # after a successful upload).
    cache_home = Path.home() / ".cache"
    cache_home.mkdir(parents=True, exist_ok=True)
    env["XDG_CACHE_HOME"] = str(cache_home)
    local_cache = ROOT / "node_modules" / ".cache" / "wrangler"
    local_cache.mkdir(parents=True, exist_ok=True)
    cmd = [
        "npx", "--yes", f"wrangler@{WRANGLER_VERSION}",
        "pages", "deploy", str(PUBLIC),
        "--project-name", cfg["PROJECT"],
        "--branch", "main",
        "--commit-dirty=true",
        "--commit-message", commit_message or "ai-digest publish",
    ]
    print("deploying with wrangler pages deploy (static only, no Functions)", file=sys.stderr)
    proc = subprocess.run(
        cmd,
        env=env,
        cwd=str(ROOT),
        capture_output=True,
        text=True,
        timeout=420,
    )

    def _scrub(s: str) -> str:
        return s.replace(token, "***") if token and s else s

    stdout = _scrub(proc.stdout or "")
    stderr = _scrub(proc.stderr or "")
    if stdout:
        print(stdout, end="" if stdout.endswith("\n") else "\n", file=sys.stderr)
    combined = stdout + "\n" + stderr
    if proc.returncode != 0:
        cache_err = (
            "permission error occurred while accessing the file system" in combined.lower()
            or "/node_modules/.cache/wrangler" in combined
        )
        uploaded = (
            "success! uploaded" in combined.lower()
            or "deployment complete" in combined.lower()
        )
        if cache_err and uploaded:
            print(
                "wrangler hit a local cache permission error after a successful "
                "upload; treating the deploy as succeeded.",
                file=sys.stderr,
            )
        else:
            raise RuntimeError(
                f"wrangler pages deploy exited {proc.returncode}\n{stderr}\n{stdout}"
            )
    elif stderr:
        print(stderr, end="" if stderr.endswith("\n") else "\n", file=sys.stderr)

    url = None
    for line in (stdout + "\n" + stderr).splitlines():
        if "https://" in line and ("pages.dev" in line or cfg["DOMAIN"] in line):
            for part in line.split():
                if part.startswith("https://"):
                    url = part.strip()
    return {"ok": True, "via": "wrangler", "result": {"url": url}, "stdout": stdout}


def deploy_with_token(cfg: dict[str, str], token: str, commit_message: str) -> dict:
    """Create the Pages project if needed, then deploy via Wrangler.

    Falls back to the raw Pages asset API only if Wrangler cannot run.
    """
    account = cfg.get("ACCOUNT_ID", "").strip()
    project = cfg.get("PROJECT", "").strip()
    if not account:
        die(f"ACCOUNT_ID not set: set env CF_ACCOUNT_ID or edit {CONFIG_PATH}")
    if not project:
        die(f"PROJECT not set: set env CF_PAGES_PROJECT or edit {CONFIG_PATH}")
    try:
        cf_request(token, "GET", f"/accounts/{account}/pages/projects/{project}")
    except RuntimeError as e:
        if "404" in str(e) or "not found" in str(e).lower():
            print(f"creating Pages project {project}", file=sys.stderr)
            cf_request(
                token,
                "POST",
                f"/accounts/{account}/pages/projects",
                body={"name": project, "production_branch": "main"},
            )
        else:
            print(f"project lookup: {e}", file=sys.stderr)
    try:
        return deploy_with_wrangler(cfg, token, commit_message)
    except FileNotFoundError:
        print("npx/wrangler not found; falling back to Pages API", file=sys.stderr)
    except RuntimeError as e:
        msg = str(e).lower()
        if "no such file" in msg or ("not found" in msg and "wrangler" in msg):
            print(f"wrangler unavailable ({e}); falling back to Pages API", file=sys.stderr)
        else:
            raise
    return _deploy_via_pages_api(cfg, token, commit_message)


def _deploy_via_pages_api(cfg: dict[str, str], token: str, commit_message: str) -> dict:
    account = cfg["ACCOUNT_ID"]
    project = cfg["PROJECT"]
    tok = cf_request(
        token, "GET", f"/accounts/{account}/pages/projects/{project}/upload-token"
    )
    jwt = (tok.get("result") or {}).get("jwt") or (tok.get("result") or {}).get("token")
    if not jwt:
        jwt = tok.get("result") if isinstance(tok.get("result"), str) else None
    if not jwt:
        raise RuntimeError(f"upload-token failed: {tok}")

    manifest, files = build_manifest()
    hashes = list(manifest.values())

    def jwt_req(method: str, path: str, body) -> dict:
        return cf_request(jwt, method, path, body=body)

    missing = hashes
    try:
        checked = jwt_req("POST", "/pages/assets/check-missing", {"hashes": hashes})
        result = checked.get("result", checked)
        if isinstance(result, list):
            missing = [x["hash"] if isinstance(x, dict) else x for x in result]
        elif isinstance(result, dict) and "hashes" in result:
            missing = result["hashes"]
    except RuntimeError as e:
        print(f"check-missing failed ({e}); uploading all files", file=sys.stderr)
        missing = hashes
    missing_set = set(missing)
    to_upload = []
    for meta in files.values():
        if meta["hash"] in missing_set:
            to_upload.append(
                {
                    "key": meta["hash"],
                    "value": meta["base64"],
                    "base64": True,
                    "metadata": {"contentType": meta["contentType"]},
                }
            )
    for i in range(0, len(to_upload), 20):
        chunk = to_upload[i : i + 20]
        jwt_req("POST", "/pages/assets/upload", chunk)
    if hashes:
        try:
            jwt_req("POST", "/pages/assets/upsert-hashes", {"hashes": hashes})
        except RuntimeError as e:
            print(f"upsert-hashes skipped: {e}", file=sys.stderr)

    boundary = "----DigestBoundary" + secrets.token_hex(16)
    parts: list[bytes] = []

    def add_field(name: str, value: str, filename: str | None = None, ctype: str | None = None):
        disp = f'Content-Disposition: form-data; name="{name}"'
        if filename:
            disp += f'; filename="{filename}"'
        headers = [disp]
        if ctype:
            headers.append(f"Content-Type: {ctype}")
        part = (
            f"--{boundary}\r\n" + "\r\n".join(headers) + "\r\n\r\n"
        ).encode("utf-8") + value.encode("utf-8") + b"\r\n"
        parts.append(part)

    add_field("manifest", json.dumps(manifest), ctype="application/json")
    add_field("branch", "main")
    add_field("commit_message", commit_message)
    headers_txt = (PUBLIC / "_headers").read_text(encoding="utf-8")
    add_field("_headers", headers_txt, filename="_headers", ctype="text/plain")
    redirects_path = PUBLIC / "_redirects"
    if redirects_path.is_file():
        redirects_txt = redirects_path.read_text(encoding="utf-8")
        add_field("_redirects", redirects_txt, filename="_redirects", ctype="text/plain")
    body = b"".join(parts) + f"--{boundary}--\r\n".encode("ascii")
    dep = cf_request(
        token,
        "POST",
        f"/accounts/{account}/pages/projects/{project}/deployments",
        form=body,
        form_content_type=f"multipart/form-data; boundary={boundary}",
    )
    if isinstance(dep, dict):
        dep.setdefault("via", "pages-api")
    return dep


def bind_custom_domain(cfg: dict[str, str], token: str | None) -> dict | None:
    """Add the Pages custom domain (cfg DOMAIN) if not already present.

    When token is None this is a no-op .
    """
    if not token:
        return None
    account = cfg["ACCOUNT_ID"]
    project = cfg["PROJECT"]
    domain = cfg["DOMAIN"]
    existing = cf_request(
        token, "GET", f"/accounts/{account}/pages/projects/{project}/domains"
    )
    names = [
        (d.get("name") or d.get("domain") or "")
        for d in (existing.get("result") or [])
    ]
    if domain in names:
        print(f"custom domain already attached: {domain}", file=sys.stderr)
        return existing
    print(f"attaching custom domain {domain}", file=sys.stderr)
    return cf_request(
        token,
        "POST",
        f"/accounts/{account}/pages/projects/{project}/domains",
        body={"name": domain},
    )


def public_urls(cfg: dict[str, str], issues: list[Issue], *, ctx: SiteContext | None = None) -> list[tuple[str, str]]:
    c = ctx or _default_ctx()
    base = f"https://{cfg['DOMAIN']}{_url_prefix(cfg['SLUG'])}"
    en_issues = list_en_sources(ctx=c) if c.lang == "zh-CN" else []
    has_lang_paths = bool(en_issues)
    out = [("index", f"{base}/")]
    if has_lang_paths:
        out.append(("zh/index", f"{base}/zh/"))
        out.append(("en/index", f"{base}/en/"))
    for i in sorted(issues, key=lambda x: (x.kind, x.start, x.label)):
        if has_lang_paths:
            out.append((f"zh/{i.kind}/{i.label}", f"{base}/zh/{i.kind}/{i.label}"))
        else:
            out.append((f"{i.kind}/{i.label}", f"{base}/{i.kind}/{i.label}"))
    for i in sorted(en_issues, key=lambda x: (x.kind, x.start, x.label)):
        out.append((f"en/{i.kind}/{i.label}", f"{base}/en/{i.kind}/{i.label}"))
    return out


_PASSWORD_OVERRIDE: str | None = None  # demo mode only (never persisted)
_SALT_OVERRIDE: bytes | None = None
ENC_CACHE_PATH = SITE / "enc-cache.json"
CHANGED_PAGES: list[str] = []


def read_password(ctx: SiteContext | None = None) -> str:
    c = ctx or _default_ctx()
    return site_crypto.read_password(c.password_override)


def load_or_create_salt(ctx: SiteContext | None = None) -> bytes:
    c = ctx or _default_ctx()
    return site_crypto.load_or_create_salt(c.salt_override)


def localize_template(
    tmpl: str,
    lang: str | None = None,
    extras: dict[str, str] | None = None,
    *,
    ctx: SiteContext | None = None,
) -> str:
    c = ctx or _default_ctx()
    return site_crypto.localize_template(tmpl, lang=lang, extras=extras, default_lang=c.lang)


def _shell_lang_for_rel(rel: str, *, ctx: SiteContext | None = None) -> str:
    c = ctx or _default_ctx()
    return site_crypto._shell_lang_for_rel(rel, default_lang=c.lang)


def shell_extras(
    rel: str = "",
    *,
    allow_lang_switch: bool = True,
    shell_lang: str | None = None,
    ctx: SiteContext | None = None,
) -> dict[str, str]:
    c = ctx or _default_ctx()
    return site_crypto.shell_extras(
        rel,
        allow_lang_switch=allow_lang_switch,
        shell_lang=shell_lang,
        default_lang=c.lang,
        lang_alt=c.lang_alt,
        password_override=c.password_override,
    )


def wrap_encrypted(
    plaintext: str,
    key: bytes,
    salt: bytes,
    tmpl: str,
    rel: str = "",
    *,
    allow_lang_switch: bool = True,
    shell_lang: str | None = None,
    ctx: SiteContext | None = None,
) -> str:
    c = ctx or _default_ctx()
    return site_crypto.wrap_encrypted(
        plaintext,
        key,
        salt,
        tmpl,
        rel,
        allow_lang_switch=allow_lang_switch,
        shell_lang=shell_lang,
        default_lang=c.lang,
        lang_alt=c.lang_alt,
        password_override=c.password_override,
    )


def _distinctive_snippets(plain: str, tmpl: str) -> list[str]:
    return site_crypto._distinctive_snippets(plain, tmpl)


def _assert_no_plaintext_leak(
    plain: str,
    wrapped: str,
    rel: Path,
    tmpl: str,
    *,
    allow_lang_switch: bool = True,
    shell_lang: str | None = None,
    ctx: SiteContext | None = None,
) -> None:
    c = ctx or _default_ctx()
    site_crypto.assert_no_plaintext_leak(
        plain,
        wrapped,
        rel,
        tmpl,
        allow_lang_switch=allow_lang_switch,
        shell_lang=shell_lang,
        default_lang=c.lang,
        lang_alt=c.lang_alt,
        password_override=c.password_override,
    )


_ROOT_CONTROL_HTML = frozenset({"404.html"})


def _prune_empty_dirs(root: Path) -> None:
    site_crypto._prune_empty_dirs(root)


def publish_plain_pages(cfg: dict[str, str], ctx: SiteContext | None = None) -> None:
    """Publish plaintext HTML directly from site/plain/ into site/public/ (ENCRYPT=0)."""
    c = ctx or _default_ctx()
    slug = cfg["SLUG"]
    src_root = _site_dir(c.plain, slug)
    dst_root = _site_dir(c.public, slug)
    if not src_root.is_dir():
        die("plaintext pages not found under site/plain; run a rebuild first")
    html_files = sorted(src_root.rglob("*.html"))
    if not html_files:
        die("no plaintext HTML to publish")

    written = reused = 0
    c.changed_pages.clear()

    def _copy_one(src: Path, dest: Path, change_name: str) -> None:
        nonlocal written, reused
        body = src.read_bytes()
        if dest.is_file() and dest.read_bytes() == body:
            reused += 1
            return
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(body)
        c.changed_pages.append(change_name)
        written += 1

    for src in html_files:
        rel = src.relative_to(src_root)
        _copy_one(src, dst_root / rel, rel.as_posix())

    keep = {s.relative_to(src_root).as_posix() for s in html_files}
    if dst_root.is_dir():
        for pub in sorted(dst_root.rglob("*.html")):
            r = pub.relative_to(dst_root).as_posix()
            if not slug and r in _ROOT_CONTROL_HTML:
                continue
            if r not in keep:
                pub.unlink()
                c.changed_pages.append(f"-{r}")

    if slug and not c.demo_mode:
        for rel_str in _ROOT_LATEST_RELS:
            if rel_str.startswith(f"{slug}/"):
                continue
            src = c.plain / rel_str
            dest = c.public / rel_str
            if src.is_file():
                _copy_one(src, dest, f"/{rel_str}")
            elif dest.is_file():
                dest.unlink()
                c.changed_pages.append(f"-/{rel_str}")

    c.enc_cache_path.unlink(missing_ok=True)
    _prune_empty_dirs(c.public)
    print(
        f"published {written} public HTML page(s), kept {reused} unchanged (ENCRYPT=0)",
        file=sys.stderr,
    )


def assert_public_matches_plain(cfg: dict[str, str], ctx: SiteContext | None = None) -> None:
    c = ctx or _default_ctx()
    site_crypto.assert_public_matches_plain(
        cfg,
        plain_dir=c.plain,
        public_dir=c.public,
        demo_mode=c.demo_mode,
        root_latest_rels=_ROOT_LATEST_RELS,
    )


def encrypt_all_pages(cfg: dict[str, str], ctx: SiteContext | None = None) -> None:
    c = ctx or _default_ctx()
    changed = site_crypto.encrypt_all_pages(
        cfg,
        plain_dir=c.plain,
        public_dir=c.public,
        template_path=c.template_path,
        enc_cache_path=c.enc_cache_path,
        default_lang=c.lang,
        lang_alt=c.lang_alt,
        demo_mode=c.demo_mode,
        password_override=c.password_override,
        salt_override=c.salt_override,
        root_latest_rels=_ROOT_LATEST_RELS,
    )
    c.changed_pages[:] = changed


def _load_enc_cache(ctx: SiteContext | None = None) -> dict[str, str]:
    return site_crypto._load_enc_cache((ctx or _default_ctx()).enc_cache_path)


def _save_enc_cache(cache: dict[str, str], ctx: SiteContext | None = None) -> None:
    site_crypto._save_enc_cache((ctx or _default_ctx()).enc_cache_path, cache)


def assert_public_is_encrypted(cfg: dict[str, str], ctx: SiteContext | None = None) -> None:
    c = ctx or _default_ctx()
    site_crypto.assert_public_is_encrypted(
        cfg,
        plain_dir=c.plain,
        public_dir=c.public,
        template_path=c.template_path,
        default_lang=c.lang,
        lang_alt=c.lang_alt,
        demo_mode=c.demo_mode,
        password_override=c.password_override,
        root_latest_rels=_ROOT_LATEST_RELS,
    )


def print_urls(cfg: dict[str, str], issues: list[Issue], *, ctx: SiteContext | None = None) -> None:
    for name, url in public_urls(cfg, issues, ctx=ctx):
        print(f"{name}: {url}")


def prepare(
    html_path: Path | None,
    kind: str | None,
    label: str | None,
    *,
    en_html_path: Path | None = None,
    ctx: SiteContext | None = None,
) -> tuple[dict[str, str], list[Issue]]:
    c = ctx or _default_ctx()
    cfg = init_config_if_needed(ctx=c)
    if html_path is not None:
        if not html_path.is_file():
            die(f"HTML file not found: {html_path}")
        if kind not in ("daily", "weekly"):
            die("type must be daily or weekly")
        if kind == "daily":
            parse_daily_label(label)  # type: ignore[arg-type]
        else:
            parse_weekly_label(label)  # type: ignore[arg-type]
        save_source(html_path, kind, label, ctx=c)  # type: ignore[arg-type]
        print(f"saved source {kind}/{label}", file=sys.stderr)
        if en_html_path is not None:
            if not en_html_path.is_file():
                die(f"English HTML file not found: {en_html_path}")
            save_source(en_html_path, kind, label, lang_sub="en", ctx=c)  # type: ignore[arg-type]
            print(f"saved English source en/{kind}/{label}", file=sys.stderr)
    issues = rebuild_all(cfg, ctx=c)
    # Privacy scan MUST run on plaintext, before encryption. Ciphertext is
    # base64 and can false-positive the email regex.
    privacy_scan(iter_plain_files(c) + iter_unencrypted_public(c), ctx=c)
    if _parse_bool(cfg.get("ENCRYPT")):
        encrypt_all_pages(cfg, ctx=c)
        _finalize_csp_headers(c)
        assert_public_is_encrypted(cfg, ctx=c)
    else:
        publish_plain_pages(cfg, ctx=c)
        _finalize_csp_headers(c)
        assert_public_matches_plain(cfg, ctx=c)
    idx_path = f"{_url_prefix(cfg['SLUG'])}/"
    print(f"prepared {len(issues)} issue(s); index at {idx_path}", file=sys.stderr)
    return cfg, issues


AUTH_HELP = f"""No Cloudflare API token found, so the site was prepared but not deployed.

Asset upload needs a scoped Cloudflare API token on this machine.

  1. Open https://dash.cloudflare.com/profile/api-tokens
  2. Create a Custom token named e.g. "ai-digest-pages" with:
       Account · Cloudflare Pages · Edit
         (your CF_ACCOUNT_ID)
       Zone · DNS · Edit, limited to your zone
         (only needed the first time, to bind SITE_DOMAIN;
          later publishes only need Pages Edit)
  3. umask 077 && install -m 600 /dev/stdin {TOKEN_FILE_DEFAULT}
     (paste the token, then Ctrl-D). Or export CLOUDFLARE_API_TOKEN.
     Do not put the token in CONFIG, the site tree, chat, or git.
  4. Re-run: python3 publish.py --deploy-only

That command uses Wrangler to create the Pages project if needed, upload
the static tree (no Functions, no Worker script), and print the public
URLs. After a successful first deploy, if DNS was not bound automatically,
attach the domain in the dashboard or via the API:
  POST /accounts/<CF_ACCOUNT_ID>/pages/projects/<CF_PAGES_PROJECT>/domains
  body {{"name": "<SITE_DOMAIN>"}}
"""


def rerender_all_sources() -> None:
    """Re-render each existing issue from its markdown into site/sources.

    When LANG=zh-CN and a companion *.en.md exists, renders separate single-lang
    Chinese (site/sources/) and English (site/sources/en/) pages so the site serves
    symmetric /zh/ and /en/ URL trees. Skips (loudly) any missing markdown.
    """
    if CONFIG_PATH.exists() or os.environ.get("SITE_SLUG", "").strip():
        load_config()
    if str(CODE_ROOT) not in sys.path:
        sys.path.insert(0, str(CODE_ROOT))
    import render_html

    (ROOT / "html").mkdir(exist_ok=True)
    for issue in list_sources():
        prefix = "digest" if issue.kind == "daily" else "weekly"
        md = ROOT / f"{prefix}-{issue.label}.md"
        if not md.is_file():
            print(f"rerender: SKIP {issue.kind}/{issue.label} (no {md.name})", file=sys.stderr)
            continue
        en_md = _discover_en_md(md)
        out = ROOT / "html" / f"{issue.kind}-{issue.label}.html"
        if en_md is not None and LANG == "zh-CN":
            render_html.render_file(md, out, LANG, year=issue.start.year)
            save_source(out, issue.kind, issue.label)
            en_out = ROOT / "html" / f"{issue.kind}-{issue.label}.en.html"
            render_html.render_file(en_md, en_out, "en", year=issue.start.year)
            save_source(en_out, issue.kind, issue.label, lang_sub="en")
            extra = f" + en/{issue.kind}/{issue.label} ({en_md.name})"
        else:
            render_html.render_file(md, out, LANG, year=issue.start.year, en_md_path=en_md)
            save_source(out, issue.kind, issue.label)
            if LANG == "zh-CN":
                (SOURCES / "en" / issue.kind / f"{issue.label}.html").unlink(missing_ok=True)
            extra = f" + {en_md.name}" if en_md else ""
        print(f"rerender: {issue.kind}/{issue.label} <- {md.name}{extra}", file=sys.stderr)


# ---------------------------------------------------------------------------
# Demo build (CI-friendly): examples/*.md -> static site in --out DIR.
# Non-interactive, no Cloudflare, nothing written outside the temp workdir and
# the output dir. Public by default (ENCRYPT=0); set env DIGEST_DEMO_PASSWORD
# (>= 4 chars; a *public* demo password) to build an encrypted demo instead.
# ---------------------------------------------------------------------------

DEMO_DAILY_LABEL = "2026-01-15"
DEMO_WEEKLY_LABEL = "2026-01-12_01-18"


def _verify_demo_pages(out_slug_dir: Path, plain_slug_dir: Path, key: bytes | None) -> int:
    """Verify every output page matches its plaintext (decrypting first when encrypted)."""
    import gzip
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM

    n = 0
    for page in sorted(out_slug_dir.rglob("*.html")):
        text = page.read_text(encoding="utf-8")
        expect = (plain_slug_dir / page.relative_to(out_slug_dir)).read_text(encoding="utf-8")
        if key is None:
            if PROMPT_MARK in text or text != expect:
                die(f"demo verify failed: {page} does not match plaintext")
        else:
            i = text.index('id="c">') + len('id="c">')
            cfg = json.loads(text[i:text.index("</script>", i)])
            packed = AESGCM(key).decrypt(
                base64.b64decode(cfg["iv"]), base64.b64decode(cfg["ct"]), None
            )
            plain = gzip.decompress(packed).decode("utf-8") if cfg.get("z") == "gzip" else packed.decode("utf-8")
            if plain != expect:
                die(f"demo verify failed: {page} decrypts to different plaintext")
        n += 1
    return n


DEMO_LANGS = ("en", "zh-CN")


def build_demo(out: Path, examples: Path, daily_label: str, weekly_label: str,
               langs: tuple[str, ...] | None = None) -> int:
    """Build the demo site from examples/ into `out` (no deploy) using isolated SiteContext.

    Public (no password) by default; set DIGEST_DEMO_PASSWORD (>= 4 chars) to
    enable client-side AES-256-GCM encryption on the demo build.
    `langs=None` or one language: a single site at /<slug>/ (slug "demo" by default).
    Both languages (DEMO_LANGS): English at /en/ and Chinese at /zh-CN/ in one output tree,
    with a language-switch link on every page and `/` redirecting to /en/.
    Zero module-global or os.environ mutations.
    """
    import shutil
    import tempfile

    password = os.environ.get("DIGEST_DEMO_PASSWORD", "")
    if 0 < len(password) < 4:
        die("DIGEST_DEMO_PASSWORD must be >= 4 chars (or unset for a public demo)")
    encrypted = bool(password)
    langs = tuple(langs) if langs else (LANG,)
    multi = len(langs) > 1
    daily_date = parse_daily_label(daily_label)
    weekly_start, _ = parse_weekly_label(weekly_label)
    for lg in langs:
        if not (examples / f"sample-digest.{lg}.md").is_file():
            die(f"missing {examples / f'sample-digest.{lg}.md'}")

    saved_slug = os.environ.get("SITE_SLUG")
    env_overrides: dict[str, str | None] = {
        "DIGEST_LANG_ALT": None,
        "DIGEST_ROOT_LATEST": None,
        "DIGEST_ENCRYPT": None,
    }
    if multi:
        env_overrides["SITE_SLUG"] = None

    work = Path(tempfile.mkdtemp(prefix="ai-digest-demo-"))
    try:
        demo_site = work / "site"
        demo_public = demo_site / "public"
        demo_plain = demo_site / "plain"
        demo_config = demo_site / "CONFIG"
        demo_template = TEMPLATE_PATH

        if encrypted:
            demo_pw: str | None = password
            demo_salt: bytes | None = os.urandom(SALT_LEN)
            key: bytes | None = derive_key(password, demo_salt)
        else:
            demo_pw = None
            demo_salt = None
            key = None

        if str(CODE_ROOT) not in sys.path:
            sys.path.insert(0, str(CODE_ROOT))
        import render_html

        slugs = {lg: (lg if multi else (saved_slug or "demo")) for lg in langs}
        total_issues = total_pages = 0
        for lg in langs:
            other = next((o for o in langs if o != lg), None)
            lang_alt = {"code": other, "base": f"/{slugs[other]}/"} if multi and other else None
            ctx = SiteContext(
                root=work,
                site=demo_site,
                public=demo_public,
                plain=demo_plain,
                sources=demo_site / f"sources-{lg}",
                config_path=demo_config,
                template_path=demo_template,
                enc_cache_path=demo_site / f"enc-cache-{lg}.json",
                lang=lg,
                cli_lang=lg,
                lang_alt=lang_alt,
                demo_mode=True,
                password_override=demo_pw,
                salt_override=demo_salt,
                env_overrides=env_overrides,
            )
            cfg_data = {
                "PLATFORM": PLATFORM,
                "PROJECT": os.environ.get("CF_PAGES_PROJECT") or PROJECT_DEFAULT,
                "ACCOUNT_ID": ACCOUNT_ID_DEFAULT,
                "DOMAIN": os.environ.get("SITE_DOMAIN") or DOMAIN_DEFAULT,
                "SLUG": slugs[lg],
                "ENCRYPT": "1" if encrypted else "0",
                "LANG": lg,
            }
            if multi and other:
                cfg_data["LANG_ALT"] = f"{other}:/{slugs[other]}/"
            write_config(cfg_data, ctx=ctx)
            html_dir = work / f"html-{lg}"
            sample_daily = examples / f"sample-digest.{lg}.md"
            sample_weekly = examples / f"sample-weekly.{lg}.md"
            jobs = [(sample_daily, "daily", daily_label, daily_date.year)]
            if sample_weekly.is_file():
                jobs.append((sample_weekly, "weekly", weekly_label, weekly_start.year))
            for md, kind, label, issue_year in jobs:
                h = html_dir / f"{kind}-{label}.html"
                render_html.render_file(md, h, lg, year=issue_year)
                save_source(h, kind, label, ctx=ctx)
            cfg = load_config(ctx)
            issues = rebuild_all(cfg, ctx=ctx)
            privacy_scan(iter_plain_files(ctx) + iter_unencrypted_public(ctx), ctx=ctx)
            if encrypted:
                encrypt_all_pages(cfg, ctx=ctx)
                assert_public_is_encrypted(cfg, ctx=ctx)
            else:
                publish_plain_pages(cfg, ctx=ctx)
                assert_public_matches_plain(cfg, ctx=ctx)
            total_pages += _verify_demo_pages(demo_public / slugs[lg], demo_plain / slugs[lg], key)
            total_issues += len(issues)
        entry = slugs[langs[0]]
        # Pages: send the site root to the (first language's) index; per-language latest aliases
        # are static HTML pages served directly at HTTP 200 (URL stays on latest).
        (demo_public / "_redirects").write_text(f"/ /{entry}/ 302\n", encoding="utf-8")
        if multi:  # one _headers with cache rules for every language prefix
            rules = "".join(_slug_cache_headers(sl) for sl in slugs.values())
            (demo_public / "_headers").write_text(rules, encoding="utf-8")
        site_crypto.finalize_csp_headers(demo_public, demo_plain)
        if out.exists():
            shutil.rmtree(out)
        shutil.copytree(demo_public, out)
        where = ", ".join(f"/{sl}/" for sl in slugs.values())
        if encrypted:
            print(f"demo site: {total_issues} issue(s), {total_pages} page(s) verified by decrypt -> {out}/ (entry: /{entry}/; {where})")
        else:
            print(f"demo site: {total_issues} issue(s), {total_pages} public page(s) verified -> {out}/ (entry: /{entry}/; {where})")
        return 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


_PUBLISH_LOCK = None


def _acquire_publish_lock(max_wait: int = 900) -> None:
    """Serialize publishes (daily 00:30 vs weekly 01:30 on Mondays, or a manual
    --rerender-all): site/ trees, enc-cache.json and the deploy must not interleave.
    Held until the process exits. (2026-11-18 review)"""
    global _PUBLISH_LOCK
    import fcntl
    ROOT.mkdir(parents=True, exist_ok=True)
    _PUBLISH_LOCK = open(ROOT / ".publish.lock", "w")
    t0 = time.time()
    while True:
        try:
            fcntl.flock(_PUBLISH_LOCK, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return
        except BlockingIOError:
            if time.time() - t0 > max_wait:
                die(f"another publish.py is still running after {max_wait // 60} min; re-run the same command later")
            if time.time() - t0 < 1:
                print("wait: another publish.py is running; waiting for it to finish…", file=sys.stderr)
            time.sleep(5)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--version", action="version", version=f"ai-digest {__version__}")
    parser.add_argument("html", nargs="?", help="path to rendered HTML")
    parser.add_argument("kind", nargs="?", choices=["daily", "weekly"])
    parser.add_argument("date_label", nargs="?", help="daily: YYYY-MM-DD; weekly: YYYY-MM-DD_MM-DD")
    parser.add_argument("--rebuild", action="store_true", help="rebuild nav, index, and static files from sources")
    parser.add_argument("--prepare-only", action="store_true", help="do not deploy")
    parser.add_argument("--deploy-only", action="store_true", help="deploy already-prepared public/ tree")
    parser.add_argument("--init", action="store_true", help="create CONFIG and exit (public root-mounted by default; pass --encrypt to enable password encryption + random SLUG)")
    parser.add_argument("--encrypt", action="store_true", help="with --init: enable client-side password encryption (ENCRYPT=1) and generate a random 22-char SLUG")
    parser.add_argument(
        "--rotate-password",
        action="store_true",
        help=(
            "generate a new password (same file, mode 600) and a new salt, "
            "re-encrypt all issues from sources, and deploy (unless --prepare-only; requires ENCRYPT=1). "
            "Never prints the password."
        ),
    )
    parser.add_argument("--rerender-all", action="store_true", help="re-render every issue in site/sources from its markdown (digest-LABEL.md / weekly-LABEL.md) with the current render_html.py, then rebuild + deploy (use after a template change; implies --rebuild)")
    parser.add_argument("--lang", help="output language: en | zh-CN (default: $DIGEST_LANG, else CONFIG LANG, else en). With --demo also: all (English at /en/ + Chinese at /zh-CN/; the --demo default when no language is given)")
    parser.add_argument("--demo", action="store_true", help="build the demo site from examples/ into --out (no deploy, no config/secrets needed; public by default, or set env DIGEST_DEMO_PASSWORD for encrypted demo)")
    parser.add_argument("--out", default="dist", help="with --demo: output directory (default: dist)")
    parser.add_argument("--examples", default=None, help="with --demo: examples directory (default: <repo>/examples)")
    parser.add_argument("--issue", help="incremental publish of one issue: daily/YYYY-MM-DD or weekly/YYYY-MM-DD_MM-DD")
    parser.add_argument("--md", help="with --issue: markdown to render (render_html.py) before publishing")
    parser.add_argument("--md-en", help="with --issue --md: English companion markdown for /en/ path rendering (auto-discovered as <stem>.en.md when LANG=zh-CN)")
    parser.add_argument("--remove", help="remove one issue (daily/LABEL or weekly/LABEL); backup to backfill/removed/")
    parser.add_argument("--no-verify", action="store_true", help="with --issue/--remove: skip post-deploy verification")
    parser.add_argument("--verbose", action="store_true", help="with --issue/--remove: stream full logs instead of a short summary")
    args = parser.parse_args(argv)

    demo_all = bool(args.demo and (args.lang == "all" or (not args.lang and not os.environ.get("DIGEST_LANG", "").strip())))
    demo_cli_lang = None if args.lang == "all" else args.lang
    if args.demo:
        try:
            demo_single_lang = i18n.resolve_lang(demo_cli_lang)
        except ValueError as e:
            die(str(e))
        return build_demo(
            Path(args.out),
            Path(args.examples) if args.examples else CODE_ROOT / "examples",
            DEMO_DAILY_LABEL, DEMO_WEEKLY_LABEL,
            DEMO_LANGS if demo_all else (demo_single_lang,),
        )

    global CLI_LANG
    CLI_LANG = demo_cli_lang
    set_lang(CLI_LANG)

    if args.init:
        cfg = init_config_if_needed(encrypt=args.encrypt)
        print(f"SLUG={cfg['SLUG']}")
        print(f"ENCRYPT={cfg['ENCRYPT']}")
        return 0

    _acquire_publish_lock()

    if args.md_en and not (args.issue and args.md):
        die("--md-en requires --issue and --md")

    if args.issue or args.remove:
        return run_incremental(args)

    if args.rerender_all:
        init_config_if_needed()
        rerender_all_sources()
        args.rebuild = True

    if args.rotate_password:
        cfg_cur = init_config_if_needed()
        if not _parse_bool(cfg_cur.get("ENCRYPT")):
            die("--rotate-password requires ENCRYPT=1 in site/CONFIG (or DIGEST_ENCRYPT=1)")
        rotate_password_and_salt()
        ENC_CACHE_PATH.unlink(missing_ok=True)  # full re-encrypt
        # Re-encrypt from sources (and save a new issue if one was given).
        html_path = Path(args.html) if args.html else None
        cfg, issues = prepare(html_path, args.kind, args.date_label)
    elif args.deploy_only:
        cfg = load_config()
        issues = list_sources()
        if not issues and not (_site_dir(PUBLIC, cfg["SLUG"]) / "index.html").exists():
            die("nothing to deploy; publish an issue first")
        # Never privacy-scan ciphertext (base64 can false-positive).
        is_enc = _parse_bool(cfg.get("ENCRYPT"))
        scan_paths = iter_unencrypted_public() + iter_plain_files()
        if not is_enc:
            scan_paths += [p for p in iter_public_files() if p.suffix.lower() == ".html"]
        if scan_paths:
            privacy_scan(scan_paths)
        if is_enc:
            assert_public_is_encrypted(cfg)
        else:
            assert_public_matches_plain(cfg)
    elif args.rebuild or (args.html is None and not args.deploy_only):
        if args.html:
            # rebuild plus a new issue if provided
            cfg, issues = prepare(Path(args.html), args.kind, args.date_label)
        elif args.rebuild:
            cfg, issues = prepare(None, None, None)
        else:
            parser.print_help()
            return 2
    else:
        if not (args.html and args.kind and args.date_label):
            parser.print_help()
            return 2
        cfg, issues = prepare(Path(args.html), args.kind, args.date_label)

    if args.prepare_only:
        print_urls(cfg, issues)
        print("prepared only; skipped deploy", file=sys.stderr)
        return 0

    if args.deploy_only or not args.prepare_only:
        token = find_token()
        if not token:
            print_urls(cfg, issues)
            print(AUTH_HELP, file=sys.stderr)
            return 2
        try:
            result = deploy_with_token(
                cfg, token, f"publish {args.kind or 'rebuild'} {args.date_label or ''}".strip()
            )
        except Exception as e:
            print(f"deploy failed: {e}", file=sys.stderr)
            print(
                "Site files are prepared. After a Cloudflare API token is in "
                f"{TOKEN_FILE_DEFAULT} (or CLOUDFLARE_API_TOKEN), re-run --deploy-only.",
                file=sys.stderr,
            )
            return 1
        try:
            bind_custom_domain(cfg, token)
        except Exception as e:
            print(f"custom domain bind failed (site is deployed): {e}", file=sys.stderr)
        url = None
        res = result.get("result") if isinstance(result, dict) else None
        if isinstance(res, dict):
            url = res.get("url") or (res.get("aliases") or [None])[0]
        print(f"deployed: {url or 'ok'}", file=sys.stderr)
        print_urls(cfg, issues)
        return 0

    return 0


# ---------------------------------------------------------------------------
# Incremental, quiet publish (--issue / --remove). Designed for routine runs:
# the agent writes ONE markdown file, this renders that one issue, rebuilds
# nav/index from site/sources (cheap, deterministic), writes/re-encrypts only
# pages whose plaintext changed, deploys (Wrangler uploads only new hashes),
# checks HTTP + verify, and prints a few summary lines.
# Full logs go to logs/publish-*.log, never to stdout.
# ---------------------------------------------------------------------------

LOG_DIR = ROOT / "logs"
REMOVED_DIR = ROOT / "backfill" / "removed"


def _parse_issue_ref(ref: str) -> tuple[str, str]:
    ref = ref.strip().strip("/")
    if ref.endswith(".html"):
        ref = ref[:-5]
    parts = ref.split("/")
    if len(parts) != 2 or parts[0] not in ("daily", "weekly"):
        die(f"issue must look like daily/YYYY-MM-DD or weekly/YYYY-MM-DD_MM-DD, got {ref!r}")
    kind, label = parts
    if kind == "daily":
        parse_daily_label(label)
    else:
        parse_weekly_label(label)
    return kind, label


def _http_status(url: str) -> tuple[int, str]:
    class _NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *a, **k):  # noqa: D401
            return None
    opener = urllib.request.build_opener(_NoRedirect)
    req = urllib.request.Request(url, method="GET", headers={"User-Agent": "ai-digest-check"})
    try:
        with opener.open(req, timeout=30) as r:
            return r.status, ""
    except urllib.error.HTTPError as e:
        return e.code, e.headers.get("Location", "") or ""
    except Exception as e:  # network error
        return 0, str(e)[:80]


def _verify_decrypt(log) -> tuple[bool, str]:
    last = ""
    for attempt in range(5):
        proc = subprocess.run(
            ["node", str(CODE_ROOT / "verify_decrypt.mjs"), "--url-base", f"https://{load_config()['DOMAIN']}"],
            cwd=str(ROOT), capture_output=True, text=True, timeout=300,
        )
        log.write(f"[verify attempt {attempt + 1}] rc={proc.returncode}\n{proc.stdout}\n{proc.stderr}\n")
        if proc.returncode == 0:
            m = re.search(r"all (\d+) page", proc.stdout)
            return True, f"OK ({m.group(1) if m else '?'} pages, exact match)"
        fails = [l for l in (proc.stdout + proc.stderr).splitlines() if l.startswith("FAIL") or "failed" in l]
        last = "; ".join(fails[:3])[:300]
        time.sleep(10)  # CDN propagation
    return False, f"FAILED: {last}"


def run_incremental(args) -> int:
    import contextlib
    import traceback

    LOG_DIR.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    action = "remove" if args.remove else "issue"
    ref = args.remove or args.issue
    log_path = LOG_DIR / f"publish-{stamp}-{action}.log"
    out = sys.stdout
    summary: list[str] = []
    ok = True
    log = open(log_path, "w", encoding="utf-8")
    sink = sys.stderr if args.verbose else log
    try:
        with contextlib.redirect_stdout(sink), contextlib.redirect_stderr(sink):
            kind, label = _parse_issue_ref(ref)
            cfg = init_config_if_needed()
            base = f"https://{cfg['DOMAIN']}{_url_prefix(cfg['SLUG'])}"
            items_note = ""
            if args.remove:
                src = SOURCES / kind / f"{label}.html"
                en_src = SOURCES / "en" / kind / f"{label}.html"
                bdir = REMOVED_DIR / f"{kind}-{label}-{stamp}"
                bdir.mkdir(parents=True, exist_ok=True)
                moved = []
                for p, backup_name in (
                    (src, f"source-{src.name}"),
                    (en_src, f"source-en-{en_src.name}"),
                    (ROOT / "html" / f"{kind}-{label}.html", f"{kind}-{label}.html"),
                    (ROOT / "html" / f"{kind}-{label}.en.html", f"{kind}-{label}.en.html"),
                ):
                    if p.is_file():
                        p.rename(bdir / backup_name)
                        moved.append(p.name)
                for sub in ("plain", "public"):
                    sub_dir = _site_dir(SITE / sub, cfg["SLUG"])
                    for rel_p in (
                        Path(kind) / f"{label}.html",
                        Path("zh") / kind / f"{label}.html",
                        Path("en") / kind / f"{label}.html",
                    ):
                        p = sub_dir / rel_p
                        if p.is_file():
                            if sub == "public":
                                safe_name = rel_p.as_posix().replace("/", "-")
                                (bdir / f"published-{safe_name}").write_bytes(p.read_bytes())
                            p.unlink()
                if not moved:
                    die(f"{kind}/{label} not found in site/sources")
                print(f"moved {moved} to {bdir}")
                rf = SITE / "removed-paths.txt"
                prev = rf.read_text(encoding="utf-8").split() if rf.is_file() else []
                if f"{kind}/{label}" not in prev:
                    rf.write_text("\n".join(prev + [f"{kind}/{label}"]) + "\n", encoding="utf-8")
                summary.append(f"removed: {kind}/{label} (backup: {bdir})")
                cfg, issues = prepare(None, None, None)
            else:
                html_path = ROOT / "html" / f"{kind}-{label}.html"
                en_html_path: Path | None = None
                if args.md:
                    md = Path(args.md)
                    if not md.is_absolute() and not md.is_file():
                        md = ROOT / md
                    if not md.is_file():
                        die(f"markdown not found: {md}")
                    en_md = _discover_en_md(md, Path(args.md_en) if args.md_en else None)
                    issue_year = (
                        parse_daily_label(label).year
                        if kind == "daily"
                        else parse_weekly_label(label)[0].year
                    )
                    if str(CODE_ROOT) not in sys.path:
                        sys.path.insert(0, str(CODE_ROOT))
                    import render_html

                    try:
                        if en_md is not None and LANG == "zh-CN":
                            digest = render_html.render_file(
                                md.resolve(),
                                html_path,
                                LANG,
                                year=issue_year,
                            )
                            en_html_path = ROOT / "html" / f"{kind}-{label}.en.html"
                            render_html.render_file(
                                en_md.resolve(),
                                en_html_path,
                                "en",
                                year=issue_year,
                            )
                        else:
                            digest = render_html.render_file(
                                md.resolve(),
                                html_path,
                                LANG,
                                year=issue_year,
                                en_md_path=en_md.resolve() if en_md else None,
                            )
                    except ValueError as e:
                        die(f"render_html failed: {e}")
                    items_note = f" ({len(digest.items)} items)"
                    if en_md is not None:
                        items_note += f" + English ({en_md.name})"
                else:
                    if not html_path.is_file():
                        die(f"no --md given and {html_path} does not exist")
                    cand_en = ROOT / "html" / f"{kind}-{label}.en.html"
                    if cand_en.is_file() and LANG == "zh-CN":
                        en_html_path = cand_en
                rf = SITE / "removed-paths.txt"
                if rf.is_file():
                    keep = [x for x in rf.read_text(encoding="utf-8").split() if x != f"{kind}/{label}"]
                    rf.write_text("\n".join(keep) + ("\n" if keep else ""), encoding="utf-8")
                cfg, issues = prepare(html_path, kind, label, en_html_path=en_html_path)
                summary.append(f"published: {kind}/{label}{items_note}")
            has_lang_paths = bool(list_en_sources()) and LANG == "zh-CN"
            issue_url = f"{base}/zh/{kind}/{label}" if has_lang_paths else f"{base}/{kind}/{label}"
            changed = list(CHANGED_PAGES)
            summary.append("changed pages: " + (", ".join(changed) if changed else "none"))

            if args.prepare_only:
                summary.append("deploy: skipped (--prepare-only)")
            else:
                token = find_token()
                if not token:
                    die("no Cloudflare API token on box; site prepared but not deployed")
                deploy_with_token(cfg, token, f"{action} {kind}/{label}")
                summary.append("deploy: ok")

                if args.remove:
                    st1, loc1 = _http_status(issue_url)
                    st2, loc2 = _http_status(issue_url + ".html")
                    summary.append(f"removed URL check: extensionless -> {st1}{' ' + loc1 if loc1 else ''}; .html -> {st2}{' ' + loc2 if loc2 else ''}")
                else:
                    st = 0
                    for _ in range(6):
                        st, _loc = _http_status(issue_url)
                        if st == 200:
                            break
                        time.sleep(10)
                    summary.append(f"url: {issue_url}  (HTTP {st})")
                    if st != 200:
                        ok = False
                idx_st, _ = _http_status(f"{base}/")
                summary.append(f"index: {base}/  (HTTP {idx_st})")
                if not args.no_verify:
                    vok, vmsg = _verify_decrypt(log)
                    label_chk = "decrypt check" if _parse_bool(cfg.get("ENCRYPT")) else "verify check"
                    summary.append(f"{label_chk}: {vmsg}")
                    ok = ok and vok
    except SystemExit as e:
        ok = False
        summary.append(f"FAILED (exit {e.code})")
    except Exception:
        ok = False
        log.write(traceback.format_exc())
        summary.append("FAILED (exception; see log)")
    finally:
        log.close()
    if not ok:
        tail = log_path.read_text(encoding="utf-8", errors="replace").splitlines()[-15:]
        summary.append("log tail:\n  " + "\n  ".join(t[:200] for t in tail))
    summary.append(f"log: {log_path}")
    print(("OK " if ok else "ERROR ") + "\n".join(summary), file=out)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
