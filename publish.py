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
import gzip
import hashlib
import html as htmlmod
import json
import mimetypes
import os
import re
import secrets
import stat
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import date, datetime, timedelta
from pathlib import Path
from typing import Iterable
from zoneinfo import ZoneInfo

from cryptography.hazmat.primitives.ciphers.aead import AESGCM

import i18n
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
PASSWORD_FILE_DEFAULT = Path.home() / ".config/ai-digest/site-password"
SALT_FILE_DEFAULT = Path.home() / ".config/ai-digest/site-salt"
PASSWORD_ALPHABET = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789"  # gitleaks:allow (character set, not a secret)
PBKDF2_ITERATIONS = 600_000
SALT_LEN = 16
NONCE_LEN = 12
KEY_LEN = 32

# Defaults when optional keys are unset in site/CONFIG or env (see load_config).
ACCOUNT_ID_DEFAULT = ""
ZONE_ID_DEFAULT = ""
PROJECT_DEFAULT = "ai-digest"
DOMAIN_DEFAULT = "digest.example.com"
PLATFORM = "pages"
CF_API = "https://api.cloudflare.com/client/v4"
WRANGLER_VERSION = os.environ.get("WRANGLER_VERSION", "4.166.0").strip() or "4.166.0"
SECURITY_HEADERS = (
    "/*\n"
    "  X-Robots-Tag: noindex, nofollow\n"
    "  X-Content-Type-Options: nosniff\n"
    "  Referrer-Policy: no-referrer\n"
    "  X-Frame-Options: DENY\n"
    "  Content-Security-Policy: default-src 'none'; script-src 'unsafe-inline'; "
    "style-src 'unsafe-inline'; img-src 'self' data:; font-src 'self'; "
    "connect-src 'self'; prefetch-src 'self'; frame-ancestors 'none'; base-uri 'none'; "
    "form-action 'none'; object-src 'none'\n"
)

# Output language of the site (en | zh-CN); resolved by load_config() / --lang, see i18n.py.
LANG = i18n.DEFAULT_LANG
CLI_LANG: str | None = None
_DEMO_MODE: bool = False
# Companion site in the other language, e.g. {"code": "zh-CN", "base": "/zh-CN/"}.
# Set via LANG_ALT in site/CONFIG (or DIGEST_LANG_ALT env), or automatically in --demo.
LANG_ALT: dict[str, str] | None = None

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
# Text that every encrypted prompt shell must contain (used by sanity checks).
PROMPT_MARK = 'id="rm"'  # the "remember this device" checkbox; language-neutral
STORE_KEY = "aie.k"

# Generic defaults. Add your own (names, handles, private ids, ...) one regex per
# line in site/privacy-patterns.txt (or the file named by DIGEST_PRIVACY_PATTERNS_FILE);
# that file is gitignored. Format: `regex` or `regex<TAB>label`; '#' = comment.
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
]


def _load_extra_privacy_patterns() -> list[tuple[str, str]]:
    env = os.environ.get("DIGEST_PRIVACY_PATTERNS_FILE", "").strip()
    path = Path(env) if env else SITE / "privacy-patterns.txt"
    out: list[tuple[str, str]] = []
    if path.is_file():
        for line in path.read_text(encoding="utf-8").splitlines():
            line = line.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            pat, _, label = line.partition("\t")
            out.append((pat.strip(), label.strip() or "custom"))
    return out

NAV_CSS = """
.issue-nav {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px 16px;
  padding: 12px 0;
  font-size: 12.5px;
  letter-spacing: 0.04em;
  color: var(--ink-4);
}
.issue-nav.top { border-bottom: 1px solid var(--line); margin-bottom: 4px; }
.issue-nav.bottom {
  border-top: 1px solid var(--line);
  margin-top: 28px;
  padding: 18px 0 0;
}
.issue-nav a { color: var(--accent-ink); text-decoration: none; }
.issue-nav a:hover { text-decoration: underline; text-underline-offset: 3px; }
.issue-nav .gone { color: var(--ink-4); opacity: 0.45; }
.issue-nav .sep {
  width: 1px;
  height: 11px;
  background: var(--line-strong);
  display: inline-block;
}
.issue-nav .week-link { margin-left: auto; }
.issue-nav .lock {
  border: 0; padding: 0; background: none; font: inherit; letter-spacing: inherit;
  color: var(--ink-4); cursor: pointer;
}
.issue-nav .lock:hover { color: var(--accent-ink); }
.issue-nav .lang-switch { color: var(--ink-4); }
@media (max-width: 560px) {
  .issue-nav .week-link { margin-left: 0; }
}
""".strip()


LOCK_JS = (
    "document.addEventListener('click',function(e){var b=e.target.closest&&e.target.closest('[data-lock]');"
    "if(!b)return;e.preventDefault();try{sessionStorage.removeItem('" + STORE_KEY + "');"
    "localStorage.removeItem('" + STORE_KEY + "')}catch(_){}location.reload()});"
)
LOCK_JS_BLOCK = f"{LOCK_MARK_START}\n<script>{LOCK_JS}</script>\n{LOCK_MARK_END}"


def perf_head_html(hints: list[str], slug: str | None = None) -> str:
    """Prefetch hints + speculation rules (prerender same-site links on intent).

    Lives inside the encrypted plaintext; the decrypted document is parsed via
    document.write, so these take effect once the page is open.
    """
    slug = slug or load_config()["SLUG"]
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
                        {"href_matches": f"/{slug}/*"},
                        {"not": {"selector_matches": "[data-lock], [target=_blank]"}},
                    ]},
                    "eagerness": "moderate",
                }
            ]
        },
        separators=(",", ":"),
    )
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


def issue_prefetch_hints(issue: "Issue", issues: list["Issue"]) -> list[str]:
    same = sorted([i for i in issues if i.kind == issue.kind], key=lambda x: (x.start, x.end, x.label))
    idx = next(n for n, i in enumerate(same) if i.label == issue.label)
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


def inject_perf_and_lock(html: str, issue: "Issue", issues: list["Issue"], slug: str | None = None) -> str:
    head = perf_head_html(issue_prefetch_hints(issue, issues), slug=slug)
    html = re.sub(r"(<meta charset=[^>]*>\s*)", lambda m: m.group(1) + head, html, count=1, flags=re.I)
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
        # Plaintext after nav injection; encrypted copy is written to PUBLIC.
        return PLAIN_SLUG() / self.kind / f"{self.label}.html"

    def sort_key(self) -> tuple:
        # Newest first: later end date, then later start, daily after weekly on same day? 
        # Independent lists, so just end then start.
        return (self.end, self.start, self.label)


def PUBLIC_SLUG() -> Path:
    return PUBLIC / load_config()["SLUG"]


def PLAIN_SLUG() -> Path:
    return PLAIN / load_config()["SLUG"]


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
}

_TRUTHY_VALUES = frozenset({"1", "true", "yes", "on"})
_PLACEHOLDER_RE = re.compile(r"<[^<>]*>|\{\{[^{}]*\}\}|[<>]")
_SLUG_SAFE_RE = re.compile(r"^[A-Za-z0-9._~-]+$")


def _parse_bool(raw: str | bool | None) -> bool:
    if isinstance(raw, bool):
        return raw
    return (raw or "").strip().lower() in _TRUTHY_VALUES


def _read_config_raw() -> dict[str, str]:
    data: dict[str, str] = {}
    if CONFIG_PATH.exists():
        for line in CONFIG_PATH.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            k, v = line.split("=", 1)
            data[k.strip()] = v.strip()
    return data


def _parse_lang_alt(raw: str | None, current_lang: str) -> dict[str, str] | None:
    """Parse LANG_ALT (e.g. 'zh-CN:/zh-CN/', '/zh-slug/', 'https://zh.example.com/s/') -> {code, base}."""
    val = (raw or "").strip()
    if not val:
        return None
    m = re.match(r"^([A-Za-z][A-Za-z_-]{1,9}):(?!//)(.+)$", val)
    if m:
        code = i18n.normalize_lang(m.group(1))
        base = m.group(2).strip()
    else:
        code = "zh-CN" if i18n.normalize_lang(current_lang) == "en" else "en"
        base = val
    if not base:
        raise ValueError("empty LANG_ALT target base URL/path")
    low = base.lower()
    if low.startswith(("http://", "https://")):
        base = base.rstrip("/") + "/"
    else:
        if ":" in base.split("/", 1)[0]:
            raise ValueError(f"unsupported URL scheme in LANG_ALT {val!r}")
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


def load_config() -> dict[str, str]:
    """site/CONFIG (KEY=VALUE lines), overridden by env vars (SITE_DOMAIN,
    SITE_SLUG, CF_ACCOUNT_ID, CF_ZONE_ID, CF_PAGES_PROJECT, DIGEST_LANG,
    DIGEST_LANG_ALT, DIGEST_ROOT_LATEST)."""
    global LANG_ALT
    data: dict[str, str] = {"PLATFORM": PLATFORM}
    data.update(_read_config_raw())
    for alias, canonical in (
        ("CF_ACCOUNT_ID", "ACCOUNT_ID"),
        ("CF_PROJECT", "PROJECT"),
        ("CF_PAGES_PROJECT", "PROJECT"),
    ):
        if alias in data and canonical not in data:
            data[canonical] = data[alias]
    for env_k, key in _ENV_KEYS.items():
        v = os.environ.get(env_k, "").strip()
        if v:
            data[key] = v
    data.setdefault("PROJECT", PROJECT_DEFAULT)
    data.setdefault("DOMAIN", DOMAIN_DEFAULT)
    data.setdefault("ACCOUNT_ID", ACCOUNT_ID_DEFAULT)
    for chk_key in ("SLUG", "ACCOUNT_ID", "CF_ACCOUNT_ID", "PROJECT", "CF_PROJECT", "DOMAIN", "ZONE_ID", "LANG_ALT", "ROOT_LATEST"):
        val = data.get(chk_key, "")
        if val and _PLACEHOLDER_RE.search(val):
            die(
                f"{chk_key} contains an unreplaced placeholder ({val!r}); "
                f"run python3 publish.py --init or edit {CONFIG_PATH}"
            )
    set_lang(CLI_LANG, data.get("LANG"))
    data["LANG"] = LANG
    slug = data.get("SLUG", "").strip()
    if not slug:
        die(f"SLUG not set: run python3 publish.py --init or edit {CONFIG_PATH} (or set env SITE_SLUG)")
    if slug in {".", ".."} or not _SLUG_SAFE_RE.fullmatch(slug):
        die(
            f"invalid SLUG {slug!r}: must contain only URL-safe characters [A-Za-z0-9._~-]; "
            f"run python3 publish.py --init or edit {CONFIG_PATH}"
        )
    if not _DEMO_MODE and len(slug) < 8:
        die(
            f"SLUG {slug!r} is too short ({len(slug)} chars; must be >= 8 unless --demo); "
            f"run python3 publish.py --init or edit {CONFIG_PATH}"
        )
    data["SLUG"] = slug
    data["ROOT_LATEST"] = "1" if _parse_bool(data.get("ROOT_LATEST")) else "0"
    try:
        LANG_ALT = _parse_lang_alt(data.get("LANG_ALT"), LANG)
    except ValueError as e:
        die(f"{e}; run python3 publish.py --init or edit {CONFIG_PATH}")
    return data


def write_config(data: dict[str, str]) -> None:
    SITE.mkdir(parents=True, exist_ok=True)
    order = [
        "PLATFORM",
        "PROJECT",
        "ACCOUNT_ID",
        "ZONE_ID",
        "DOMAIN",
        "SLUG",
        "LANG",
        "LANG_ALT",
        "ROOT_LATEST",
    ]
    lines = [
        "# AI digest public site. No secrets. Do not change SLUG once published.",
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
    CONFIG_PATH.write_text("\n".join(lines) + "\n", encoding="utf-8")


def init_config_if_needed() -> dict[str, str]:
    if CONFIG_PATH.exists() or os.environ.get("SITE_SLUG", "").strip():
        return load_config()
    alphabet = "abcdefghijkmnopqrstuvwxyz23456789"
    slug = "".join(secrets.choice(alphabet) for _ in range(22))
    data = {
        "PLATFORM": PLATFORM,
        "PROJECT": os.environ.get("CF_PAGES_PROJECT") or PROJECT_DEFAULT,
        "ACCOUNT_ID": os.environ.get("CF_ACCOUNT_ID") or ACCOUNT_ID_DEFAULT,
        "ZONE_ID": os.environ.get("CF_ZONE_ID") or ZONE_ID_DEFAULT,
        "DOMAIN": os.environ.get("SITE_DOMAIN") or DOMAIN_DEFAULT,
        "SLUG": slug,
        "LANG": i18n.resolve_lang(CLI_LANG),
    }
    write_config(data)
    print(f"created {CONFIG_PATH} with slug {slug}", file=sys.stderr)
    return data


def parse_weekly_label(label: str) -> tuple[date, date]:
    if "_" not in label:
        die(f"weekly date label must look like 2026-11-09_11-13, got {label!r}")
    left, right = label.split("_", 1)
    try:
        start = date.fromisoformat(left)
    except ValueError:
        die(f"invalid weekly start date: {left!r}")
    try:
        if len(right) >= 10 and right[4] == "-":
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
    try:
        return date.fromisoformat(label)
    except ValueError:
        die(f"daily date label must be YYYY-MM-DD, got {label!r}")


def _t(key: str, **kw) -> str:
    return i18n.t(LANG, key, **kw)


def extract_title(html: str) -> str:
    m = re.search(r"<title>(.*?)</title>", html, flags=re.S | re.I)
    if not m:
        return ""
    return re.sub(r"\s+", " ", m.group(1)).strip()


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
    ):
        html = strip_marked(html, s, e)
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


def ensure_nav_css(html: str) -> str:
    html = strip_marked(html, CSS_MARK_START, CSS_MARK_END)
    block = f"{CSS_MARK_START}\n<style>\n{NAV_CSS}\n</style>\n{CSS_MARK_END}\n"
    if "</head>" not in html.lower():
        die("HTML has no </head>; cannot inject nav CSS")
    return re.sub(r"</head>", block + "</head>", html, count=1, flags=re.I)


def lang_switch_href(rel: str = "") -> str | None:
    """URL of the same page in the other language (bilingual demo), or None."""
    if not LANG_ALT:
        return None
    return LANG_ALT["base"] + rel.strip("/").removesuffix(".html").removesuffix("index")


def lang_switch_link(rel: str = "", cls: str = "lang-switch") -> str:
    href = lang_switch_href(rel)
    if href is None:
        return ""
    code = LANG_ALT["code"]  # type: ignore[index]
    return (f'<a class="{cls}" href="{_esc(href)}" hreflang="{code}" lang="{code}">'
            f'{_esc(i18n.t(code, "lang_name"))}</a>')


_LANG_ZH_RE = re.compile(r'<div\s+class="lang-pane\s+lang-zh"[^>]*>', re.I)
_LANG_EN_RE = re.compile(r'<div\s+class="lang-pane\s+lang-en"[^>]*>', re.I)


def _primary_pane_html(html: str) -> str:
    """Return the pane matching the site's primary language on bilingual pages."""
    zh_m = _LANG_ZH_RE.search(html)
    en_m = _LANG_EN_RE.search(html)
    if not (zh_m and en_m):
        return html
    if zh_m.start() < en_m.start():
        return html[en_m.start() :] if LANG == "en" else html[zh_m.start() : en_m.start()]
    return html[en_m.start() : zh_m.start()] if LANG == "en" else html[zh_m.start() :]


def _discover_en_md(md_path: Path, explicit: Path | None = None) -> Path | None:
    """Resolve an explicit --md-en path, or auto-discover sibling <stem>.en.md for zh-CN sites."""
    if explicit is not None:
        p = Path(explicit)
        if not p.is_absolute() and not p.is_file():
            p = ROOT / p
        if not p.is_file():
            die(f"English markdown not found: {p}")
        return p
    if LANG == "zh-CN" and not md_path.name.endswith(".en.md"):
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
) -> str:
    lg = i18n.normalize_lang(lang) if lang is not None else LANG

    def tr(key: str, **kw) -> str:
        return i18n.t(lg, key, **kw)

    same = sorted(
        [i for i in issues if i.kind == issue.kind],
        key=lambda x: (x.start, x.end, x.label),
    )
    idx = next(n for n, i in enumerate(same) if i.label == issue.label)
    prev = same[idx - 1] if idx > 0 else None  # 上一期 = older
    nxt = same[idx + 1] if idx + 1 < len(same) else None  # 下一期 = newer

    def link(href: str, text: str, extra: str = "") -> str:
        cls = f' class="{extra}"' if extra else ""
        return f'<a href="{href}"{cls}>{text}</a>'

    def gone(text: str) -> str:
        return f'<span class="gone">{text}</span>'

    parts = [link(slug_index_rel, tr("nav_index"))]
    parts.append('<span class="sep" aria-hidden="true"></span>')
    parts.append(link(prev.label, tr("nav_prev")) if prev else gone(tr("nav_prev")))
    parts.append(link(nxt.label, tr("nav_next")) if nxt else gone(tr("nav_next")))

    if issue.kind == "daily":
        weeklies = [
            w
            for w in issues
            if w.kind == "weekly" and w.start <= issue.start <= w.end
        ]
        weeklies.sort(key=lambda w: (w.end, w.start), reverse=True)
        if weeklies:
            parts.append(
                link(f"../weekly/{weeklies[0].label}", tr("nav_week"), "week-link")
            )

    if which == "top":
        if issue.kind != "daily" or not any(
            w.kind == "weekly" and w.start <= issue.start <= w.end for w in issues
        ):
            parts.append('<span class="week-link"></span>')
        parts.append(lang_switch_link(f"{issue.kind}/{issue.label}"))
        parts.append(f'<button type="button" class="lock" data-lock title="{_esc(tr("lock_title"))}">{_esc(tr("lock"))}</button>')
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
) -> str:
    top = (
        f"{NAV_TOP_START}\n"
        f"{nav_html(issue, issues, '../', 'top', lang=lang)}\n"
        f"{NAV_TOP_END}"
    )
    bot = (
        f"{NAV_BOT_START}\n"
        f"{nav_html(issue, issues, '../', 'bottom', lang=lang)}\n"
        f"{NAV_BOT_END}"
    )
    if "</header>" not in fragment.lower():
        die(f"{issue.kind}/{issue.label}: no </header> to place top nav")
    fragment = re.sub(r"</header>", lambda m: m.group(0) + "\n" + top, fragment, count=1, flags=re.I)
    # Place bottom nav after the end-of-issue marker, still inside the footer if present.
    end_m = re.search(r'<div class="end">[^<]*</div>', fragment)
    if end_m:
        fragment = fragment[: end_m.end()] + "\n" + bot + fragment[end_m.end():]
    elif "<footer" in fragment.lower():
        fragment = re.sub(r"(<footer[^>]*>)", lambda m: m.group(1) + "\n" + bot, fragment, count=1, flags=re.I)
    else:
        fragment = re.sub(r"</body>", lambda m: bot + "\n" + m.group(0), fragment, count=1, flags=re.I)
    return fragment


def inject_nav(html: str, issue: Issue, issues: list[Issue]) -> str:
    html = strip_marked(html, NAV_TOP_START, NAV_TOP_END)
    html = strip_marked(html, NAV_BOT_START, NAV_BOT_END)
    zh_m = _LANG_ZH_RE.search(html)
    en_m = _LANG_EN_RE.search(html)
    if zh_m and en_m:
        first_m, second_m = (zh_m, en_m) if zh_m.start() < en_m.start() else (en_m, zh_m)
        first_lang = "zh-CN" if first_m is zh_m else "en"
        second_lang = "en" if first_m is zh_m else "zh-CN"
        part1 = _inject_nav_into_fragment(html[: second_m.start()], issue, issues, lang=first_lang)
        part2 = _inject_nav_into_fragment(html[second_m.start():], issue, issues, lang=second_lang)
        return part1 + part2
    return _inject_nav_into_fragment(html, issue, issues)


def list_sources() -> list[Issue]:
    issues: list[Issue] = []
    if not SOURCES.exists():
        return issues
    for kind in ("daily", "weekly"):
        d = SOURCES / kind
        if not d.is_dir():
            continue
        for path in sorted(d.glob("*.html")):
            label = path.stem
            html = path.read_text(encoding="utf-8")
            title = extract_title(html) or f"{_t('daily_title' if kind == 'daily' else 'weekly_title')} · {label}"
            if kind == "daily":
                start = end = parse_daily_label(label)
            else:
                start, end = parse_weekly_label(label)
            issues.append(Issue(kind=kind, label=label, source=path, start=start, end=end, title=title))
    return issues


def save_source(html_path: Path, kind: str, label: str) -> Path:
    raw = html_path.read_text(encoding="utf-8")
    raw = strip_injections(raw)
    dest = SOURCES / kind / f"{label}.html"
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


def _slug_cache_headers(slug: str) -> str:
    """Per-slug Cache-Control rules; latest redirect aliases detach wildcard caching."""
    return (
        f"/{slug}/\n"
        "  Cache-Control: public, max-age=60, stale-while-revalidate=604800\n"
        f"/{slug}/daily/*\n"
        "  Cache-Control: public, max-age=300, stale-while-revalidate=604800\n"
        f"/{slug}/weekly/*\n"
        "  Cache-Control: public, max-age=300, stale-while-revalidate=604800\n"
        f"/{slug}/latest\n"
        "  Cache-Control: no-store\n"
        f"/{slug}/latest/\n"
        "  Cache-Control: no-store\n"
        f"/{slug}/daily/latest\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
        f"/{slug}/daily/latest/\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
        f"/{slug}/weekly/latest\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
        f"/{slug}/weekly/latest/\n"
        "  ! Cache-Control\n"
        "  Cache-Control: no-store\n"
    )


def _latest_issues(issues: Iterable[Issue]) -> tuple[Issue | None, Issue | None, Issue | None]:
    """Return (latest_daily, latest_weekly, latest_any) ordered by Issue.sort_key."""
    dailies = [i for i in issues if i.kind == "daily"]
    weeklies = [i for i in issues if i.kind == "weekly"]
    latest_daily = max(dailies, key=lambda i: i.sort_key()) if dailies else None
    latest_weekly = max(weeklies, key=lambda i: i.sort_key()) if weeklies else None
    latest_any = latest_daily or latest_weekly
    return latest_daily, latest_weekly, latest_any


def _latest_redirect_rules(
    slug: str,
    issues: Iterable[Issue],
    *,
    include_root: bool = False,
) -> list[str]:
    """Build pure-static Cloudflare Pages 302 redirect lines for latest issues."""
    latest_daily, latest_weekly, latest_any = _latest_issues(issues)
    rules: list[str] = []
    if latest_daily:
        target = f"/{slug}/daily/{latest_daily.label}"
        rules.append(f"/{slug}/daily/latest {target} 302")
        rules.append(f"/{slug}/daily/latest/ {target} 302")
    if latest_weekly:
        target = f"/{slug}/weekly/{latest_weekly.label}"
        rules.append(f"/{slug}/weekly/latest {target} 302")
        rules.append(f"/{slug}/weekly/latest/ {target} 302")
    if latest_any:
        target = f"/{slug}/{latest_any.kind}/{latest_any.label}"
        rules.append(f"/{slug}/latest {target} 302")
        rules.append(f"/{slug}/latest/ {target} 302")
    if include_root:
        if latest_daily:
            target = f"/{slug}/daily/{latest_daily.label}"
            rules.append(f"/daily/latest {target} 302")
            rules.append(f"/daily/latest/ {target} 302")
        if latest_weekly:
            target = f"/{slug}/weekly/{latest_weekly.label}"
            rules.append(f"/weekly/latest {target} 302")
            rules.append(f"/weekly/latest/ {target} 302")
        if latest_any:
            target = f"/{slug}/{latest_any.kind}/{latest_any.label}"
            rules.append(f"/latest {target} 302")
            rules.append(f"/latest/ {target} 302")
    return rules


def write_static_root(
    slug: str | None = None,
    issues: list[Issue] | None = None,
    *,
    root_latest: bool | None = None,
) -> None:
    PUBLIC.mkdir(parents=True, exist_ok=True)
    (PUBLIC / "robots.txt").write_text(
        "User-agent: *\nDisallow: /\n", encoding="utf-8"
    )
    if slug is None:
        cfg = load_config()
        slug = cfg["SLUG"]
        if root_latest is None:
            root_latest = _parse_bool(cfg.get("ROOT_LATEST"))
    elif root_latest is None:
        env_val = os.environ.get("DIGEST_ROOT_LATEST", "").strip()
        if env_val:
            root_latest = _parse_bool(env_val)
        else:
            root_latest = _parse_bool(_read_config_raw().get("ROOT_LATEST"))
    if issues is None:
        issues = list_sources() if SOURCES.is_dir() else []
    # HTML only changes when an issue is (re)published; short max-age +
    # long stale-while-revalidate makes back/forward and prefetched clicks
    # instant while a publish shows up within minutes. Latest redirect paths
    # come after daily/* and weekly/* and detach inherited Cache-Control so
    # they are never cached stale.
    headers_text = SECURITY_HEADERS + _slug_cache_headers(slug)
    if root_latest:
        headers_text += _ROOT_LATEST_HEADERS
    (PUBLIC / "_headers").write_text(headers_text, encoding="utf-8")
    (PUBLIC / "404.html").write_text(
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
    # Intentionally no index.html at the site root.
    # Static 302 redirects: latest issue aliases first, then removed issues
    # (sending old URLs to the index to override stale caches), deduplicated by source.
    rules: list[str] = []
    seen_sources: set[str] = set()

    def _add_rule(rule: str) -> None:
        src = rule.split(None, 1)[0]
        if src not in seen_sources:
            seen_sources.add(src)
            rules.append(rule)

    for r in _latest_redirect_rules(slug, issues, include_root=bool(root_latest)):
        _add_rule(r)
    removed = SITE / "removed-paths.txt"
    if removed.is_file():
        for line in removed.read_text(encoding="utf-8").split():
            line = line.strip().strip("/")
            if line:
                _add_rule(f"/{slug}/{line} /{slug}/ 302")
                _add_rule(f"/{slug}/{line}.html /{slug}/ 302")
    rp = PUBLIC / "_redirects"
    if rules:
        rp.write_text("\n".join(rules) + "\n", encoding="utf-8")
    elif rp.exists():
        rp.unlink()


def issue_headlines(issue: Issue) -> list[str]:
    """Item headlines of an issue, read from its clean source (TOC first)."""
    try:
        html = _primary_pane_html(issue.source.read_text(encoding="utf-8"))
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


def issue_tags(issue: Issue) -> list[str]:
    """Distinct topic tags of an issue, extracted from its rendered story badges."""
    try:
        html = _primary_pane_html(issue.source.read_text(encoding="utf-8"))
    except OSError:
        return []
    out: list[str] = []
    seen: set[str] = set()
    for cls, raw in re.findall(r'<span class="(badge[^"]*)"[^>]*>(.*?)</span>', html, flags=re.S):
        if "badge-hot" in cls:
            continue
        t = htmlmod.unescape(re.sub(r"<[^>]+>", "", raw)).strip()
        if t and t not in seen:
            seen.add(t)
            out.append(t)
    return out


def _md(d: date) -> str:
    return f"{d.month:02d}/{d.day:02d}"


def index_html(cfg: dict[str, str], issues: list[Issue]) -> str:
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
        heads_by_key[k] = issue_headlines(i)
        ts = issue_tags(i)
        tags_by_key[k] = ts
        for tg in ts:
            tag_counts[tg] = tag_counts.get(tg, 0) + 1
    top_tags = [t for t, _n in sorted(tag_counts.items(), key=lambda kv: (-kv[1], kv[0]))[:12]]

    def lead_spans(i: Issue) -> str:
        heads = heads_by_key.get((i.kind, i.label), [])
        if not heads:
            return f'<span class="lab">{_esc(i.title)}</span>'
        lead = f'<span class="lab">{_esc(heads[0])}</span>'
        if len(heads) > 1:
            lead += f'<span class="sub">{_esc(" / ".join(heads[1:3]))}</span>'
        return f'<span class="body">{lead}</span><span class="cnt num">{_esc(i18n.count_items(LANG, len(heads)))}</span>'

    def _item_attrs(i: Issue) -> str:
        k = (i.kind, i.label)
        ts = tags_by_key.get(k, [])
        heads = heads_by_key.get(k, [])
        blob = " ".join([i.label, i.title, *heads, *ts]).lower()
        return f' class="archive-item" data-tags="{_esc("|".join(ts))}" data-search="{_esc(blob)}"'

    def daily_item(i: Issue) -> str:
        return (
            f'<li{_item_attrs(i)}><a href="{i.rel_url}">'
            f'<span class="when num">{_md(i.start)}</span>'
            f'<span class="dow">{_esc(i18n.weekday_short(LANG, i.start))}</span>'
            f"{lead_spans(i)}"
            f"</a></li>"
        )

    def weekly_item(i: Issue) -> str:
        return (
            f'<li{_item_attrs(i)}><a href="{i.rel_url}">'
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
        wlink = f'<a class="wk-weekly" href="{wk.rel_url}">{_esc(_t("week_link"))}</a>' if wk else ""
        body = (
            '<ol class="issue-list">\n' + "\n".join(daily_item(i) for i in items) + "\n</ol>"
            if items else f'<p class="empty">{_esc(_t("no_daily_week"))}</p>'
        )
        week_blocks.append(
            f'<details class="grp"{" open" if n == 0 else ""}>'
            f'<summary>{chev}<span class="g-range num">{_md(mon)} – {_md(sun)}</span>'
            f'<span class="g-meta">{_esc(_t("week_n", n=mon.isocalendar()[1]))} · <span class="num">{len(items)}</span> {_esc(i18n.issue_unit(LANG, len(items)))}</span>'
            f"{wlink}</summary>\n{body}\n</details>"
        )
    daily_ol = "\n".join(week_blocks) if week_blocks else f'<p class="empty">{_esc(_t("none_yet"))}</p>'

    # Weeklies grouped by month of their end (Sunday) date, newest first.
    months: dict[tuple[int, int], list[Issue]] = {}
    for w in weeklies:
        months.setdefault((w.end.year, w.end.month), []).append(w)
    month_blocks = []
    for n, (y, m) in enumerate(sorted(months, reverse=True)):
        items = months[(y, m)]
        month_blocks.append(
            f'<details class="grp"{" open" if n == 0 else ""}>'
            f'<summary>{chev}<span class="g-range num">{_esc(i18n.month_label(LANG, y, m))}</span>'
            f'<span class="g-meta"><span class="num">{len(items)}</span> {_esc(i18n.issue_unit(LANG, len(items)))}</span></summary>\n'
            '<ol class="issue-list weekly">\n' + "\n".join(weekly_item(i) for i in items) + "\n</ol>\n</details>"
        )
    weekly_ol = "\n".join(month_blocks) if month_blocks else f'<p class="empty">{_esc(_t("none_yet"))}</p>'

    # Warm the likely next clicks: newest daily + newest weekly.
    hints = [i.rel_url for i in (dailies[:2] + weeklies[:1])]
    perf_head = perf_head_html(hints, slug=cfg["SLUG"])
    tz_name = os.environ.get("DIGEST_TZ", "").strip()
    today = datetime.now(ZoneInfo(tz_name)).date() if tz_name else datetime.now().astimezone().date()
    today_line = (
        f"{i18n.date_long(LANG, today)} · {i18n.weekday(LANG, today)}"
        if LANG == "zh-CN"
        else f"{i18n.weekday(LANG, today)}, {i18n.date_long(LANG, today)}"
    )
    tag_chips = ""
    if top_tags:
        chips = [
            f'<button type="button" class="filter-tag" data-tag="" aria-pressed="true">{_esc(_t("filter_all"))}</button>'
        ] + [
            f'<button type="button" class="filter-tag" data-tag="{_esc(tg)}" aria-pressed="false">{_esc(tg)}</button>'
            for tg in top_tags
        ]
        tag_chips = '<div class="filter-tags" role="group">' + "".join(chips) + "</div>"
    filter_bar = (
        f'<div class="filter-bar" id="archiveFilterBar" role="search" aria-label="{_esc(_t("archive_filter_label"))}"'
        f' data-total="{len(issues)}" data-tpl="{_esc(_t("filter_shown"))}">'
        '<div class="filter-row">'
        f'<input type="search" id="archiveFilterInput" class="filter-input" placeholder="{_esc(_t("archive_filter_placeholder"))}"'
        f' aria-label="{_esc(_t("archive_filter_label"))}" autocomplete="off" spellcheck="false">'
        f'<span id="archiveFilterCount" class="filter-count num" aria-live="polite" hidden></span>'
        f'<button type="button" id="archiveFilterClear" class="filter-clear" hidden>{_esc(_t("filter_clear"))}</button>'
        f"</div>{tag_chips}</div>"
        f'<p id="archiveFilterEmpty" class="filter-empty" hidden>{_esc(_t("archive_filter_empty"))}</p>'
        if issues else ""
    )
    return f"""<!DOCTYPE html>
<html lang="{_t("html_lang")}">
<head>
<meta charset="utf-8">
{META_MARK_START}
<meta name="robots" content="noindex,nofollow">
{META_MARK_END}
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<meta name="theme-color" media="(prefers-color-scheme: light)" content="#faf9f6">
<meta name="theme-color" media="(prefers-color-scheme: dark)" content="#13191c">
<title>{_esc(_t("index_title"))}</title>
<script>(function(){{try{{var t=localStorage.getItem('digest-theme');if(t==='light'||t==='dark')document.documentElement.setAttribute('data-theme',t);}}catch(e){{}}}})();</script>
{perf_head}<style>
:root {{
  --font-sans: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI",
    "PingFang SC", "Hiragino Sans GB", "HarmonyOS Sans SC", "Microsoft YaHei",
    "Noto Sans CJK SC", "Noto Sans CJK TC", "Noto Sans", sans-serif;
  --radius-panel: 14px;
  --paper-max: 1100px; /* layout2: index content capped at ~1100px, centered */
  --gutter: max(16px, env(safe-area-inset-left));
  --bg: #faf9f6;
  --bg-sunk: #f3f3ef;
  --surface: #ffffff;
  --ink: #202a30;
  --ink-2: #303c42;
  --ink-3: #59656b;
  --ink-4: #657176;
  --line: #dfe4e1;
  --line-strong: #d1d9d5;
  --line-soft: #e9ede9;
  --accent: #176b75;
  --accent-ink: #145f68;
  --well: #f3f3ef;
  --shadow-soft: 0 4px 12px rgba(28, 39, 51, 0.06);
  color-scheme: light;
}}
@media (prefers-color-scheme: dark) {{
  :root:not([data-theme="light"]) {{
    --bg: #13191c;
    --bg-sunk: #182024;
    --well: #182024;
    --surface: #1a2226;
    --ink: #e7eceb;
    --ink-2: #d2dada;
    --ink-3: #a5b0b3;
    --ink-4: #89979d;
    --line: rgba(255, 255, 255, 0.08);
    --line-strong: rgba(255, 255, 255, 0.12);
    --line-soft: rgba(255, 255, 255, 0.06);
    --accent: #12ccd8;
    --accent-ink: #2ce2e8;
    --shadow-soft: 0 6px 18px rgba(0, 0, 0, 0.35);
    color-scheme: dark;
  }}
}}
:root[data-theme="dark"] {{
  --bg: #13191c;
  --bg-sunk: #182024;
  --well: #182024;
  --surface: #1a2226;
  --ink: #e7eceb;
  --ink-2: #d2dada;
  --ink-3: #a5b0b3;
  --ink-4: #89979d;
  --line: rgba(255, 255, 255, 0.08);
  --line-strong: rgba(255, 255, 255, 0.12);
  --line-soft: rgba(255, 255, 255, 0.06);
  --accent: #12ccd8;
  --accent-ink: #2ce2e8;
  --shadow-soft: 0 6px 18px rgba(0, 0, 0, 0.35);
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
.theme-switch {{ display: inline-flex; align-items: center; gap: 2px; }}
.theme-switch button {{
  height: 26px; padding: 0 8px; border: 0; border-radius: 999px;
  background: transparent; color: var(--ink-4); font-size: 12px;
}}
.theme-switch button[aria-pressed="true"],
.theme-switch button:hover {{ color: var(--ink); background: var(--bg-sunk); }}
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
.mast-rule {{ border: 0; border-top: 1px solid var(--line-strong); margin: 0; }}
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
/* Compact masthead (2026-11-16): one row, ~1/4 of the old height */
.paper {{ padding-top: 10px; }}
@media (min-width: 700px) {{ .paper {{ padding-top: 14px; }} }}
.mast-bar {{
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 6px 0 10px;
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
  .mast-bar .theme-switch button {{ height: 24px; padding: 0 6px; font-size: 11.5px; }}
}}
.filter-bar {{
  margin: 18px 0 8px;
  padding: 10px 12px;
  background: var(--well);
  box-shadow: inset 0 0 0 1px var(--line-soft);
  border-radius: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}}
.filter-row {{ display: flex; align-items: center; gap: 8px; }}
.filter-input {{
  flex: 1 1 auto;
  min-width: 0;
  height: 32px;
  padding: 0 11px;
  border: 1px solid var(--line);
  border-radius: 8px;
  background: var(--surface);
  color: var(--ink);
  font-family: inherit;
  font-size: 13.5px;
  outline: none;
}}
.filter-input::placeholder {{ color: var(--ink-4); }}
.filter-input:focus {{ border-color: var(--accent); }}
.filter-count {{ flex: 0 0 auto; font-size: 12px; color: var(--ink-4); padding: 0 4px; }}
.filter-clear {{
  flex: 0 0 auto;
  height: 26px;
  padding: 0 10px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--surface);
  color: var(--ink-3);
  font-size: 12px;
}}
.filter-clear:hover {{ color: var(--ink); border-color: var(--accent); }}
.filter-tags {{ display: flex; flex-wrap: wrap; gap: 5px; }}
.filter-tag {{
  height: 24px;
  padding: 0 9px;
  border: 1px solid var(--line);
  border-radius: 999px;
  background: var(--surface);
  color: var(--ink-3);
  font-size: 11.5px;
}}
.filter-tag:hover {{ color: var(--accent-ink); border-color: var(--accent); }}
.filter-tag[aria-pressed="true"] {{
  background: color-mix(in srgb, var(--accent) 14%, var(--surface));
  border-color: var(--accent);
  color: var(--accent-ink);
  font-weight: 600;
}}
.filter-empty {{ margin: 20px 0 0; color: var(--ink-4); font-size: 13.5px; text-align: center; }}
@media print {{ .filter-bar, .filter-empty {{ display: none !important; }} }}
</style>
</head>
<body>
<article class="paper">
<header class="mast">
<div class="mast-bar">
<h1 class="np"><span class="ai">AI</span>{_esc(_t("brand_tail"))}</h1>
<span class="mast-chip"><span class="mark-kicker">{_esc(_t("index_chip"))}</span><span class="mark-figure num">{len(issues)}</span><span class="mark-kicker">{_esc(i18n.issue_unit(LANG, len(issues)))}</span></span>
<span class="mast-meta"><span class="num">{_esc(today_line)}</span><span>{_esc(_t("index_meta", d=len(dailies), w=len(weeklies)))}</span><span class="motto">{_esc(_t("motto"))}</span></span>
<span class="mast-tools">
{lang_switch_link("", "lang-link")}
<span class="theme-switch" role="group" aria-label="{_esc(_t("appearance"))}">
<button type="button" data-theme="system">{_esc(_t("theme_system"))}</button>
<button type="button" data-theme="light">{_esc(_t("theme_light"))}</button>
<button type="button" data-theme="dark">{_esc(_t("theme_dark"))}</button>
<button type="button" class="lock" data-lock title="{_esc(_t("lock_title"))}">{_esc(_t("lock"))}</button>
</span>
</span>
</div>
<hr class="mast-rule">
</header>
{filter_bar}
<section class="archive-sec">
<div class="kicker"><span class="bar"></span>{_esc(_t("index_daily"))}<span class="cnt num">{_esc(i18n.count_issues(LANG, len(dailies)))}</span></div>
{daily_ol}
</section>
<section class="archive-sec">
<div class="kicker"><span class="bar"></span>{_esc(_t("index_weekly"))}<span class="cnt num">{_esc(i18n.count_issues(LANG, len(weeklies)))}</span></div>
{weekly_ol}
</section>
<footer class="colophon"><div class="end">{_esc(_t("index_end"))}</div></footer>
</article>
{LOCK_JS_BLOCK}
<script>
(function(){{
  const KEY='digest-theme';
  const root=document.documentElement;
  const btns=[...document.querySelectorAll('.theme-switch [data-theme]')];
  function apply(v){{
    if(v==='light'||v==='dark') root.setAttribute('data-theme', v);
    else root.removeAttribute('data-theme');
    btns.forEach(b=>b.setAttribute('aria-pressed', String(b.getAttribute('data-theme')===v)));
  }}
  let saved='system';try{{saved=localStorage.getItem(KEY)||'system';}}catch(e){{}}
  apply(saved);
  btns.forEach(b=>b.addEventListener('click',()=>{{
    const v=b.getAttribute('data-theme');
    try{{localStorage.setItem(KEY,v);}}catch(e){{}}
    apply(v);
  }}));
  const bar=document.getElementById('archiveFilterBar');
  if(bar){{
    const inp=document.getElementById('archiveFilterInput');
    const clr=document.getElementById('archiveFilterClear');
    const cnt=document.getElementById('archiveFilterCount');
    const emp=document.getElementById('archiveFilterEmpty');
    const tbtns=[...bar.querySelectorAll('.filter-tag')];
    const items=[...document.querySelectorAll('li.archive-item')];
    const grps=[...document.querySelectorAll('details.grp')];
    const secs=[...document.querySelectorAll('section.archive-sec')];
    const initOpen=new Map(grps.map(g=>[g,g.open]));
    const tpl=bar.dataset.tpl||'{{shown}} / {{total}}';
    let curTag='';
    function runFilter(){{
      const q=(inp.value||'').trim().toLowerCase();
      const toks=q?q.split(/\\s+/):[];
      const active=Boolean(toks.length||curTag);
      let shown=0;
      for(const li of items){{
        const hay=li.dataset.search||'';
        const tags=(li.dataset.tags||'').split('|').filter(Boolean);
        const ok=(!curTag||tags.includes(curTag))&&toks.every(k=>hay.includes(k));
        li.hidden=!ok;
        if(ok)shown++;
      }}
      for(const g of grps){{
        const has=[...g.querySelectorAll('li.archive-item')].some(li=>!li.hidden);
        if(active){{
          g.hidden=!has;
          if(has)g.open=true;
        }}else{{
          g.hidden=false;
          g.open=Boolean(initOpen.get(g));
        }}
      }}
      for(const s of secs){{
        const gs=[...s.querySelectorAll('details.grp')];
        if(gs.length) s.hidden=active&&gs.every(g=>g.hidden);
      }}
      clr.hidden=!active;
      cnt.hidden=!active;
      if(active)cnt.textContent=tpl.replace('{{shown}}',String(shown)).replace('{{total}}',String(items.length));
      if(emp)emp.hidden=!(active&&shown===0);
    }}
    inp.addEventListener('input',runFilter);
    inp.addEventListener('keydown',e=>{{if(e.key==='Escape'){{inp.value='';curTag='';tbtns.forEach(b=>b.setAttribute('aria-pressed',b.dataset.tag===''?'true':'false'));runFilter();}}}});
    clr.addEventListener('click',()=>{{inp.value='';curTag='';tbtns.forEach(b=>b.setAttribute('aria-pressed',b.dataset.tag===''?'true':'false'));runFilter();inp.focus();}});
    tbtns.forEach(b=>b.addEventListener('click',()=>{{
      const tg=b.dataset.tag||'';
      curTag=(tg&&curTag===tg)?'':tg;
      tbtns.forEach(x=>x.setAttribute('aria-pressed',(x.dataset.tag||'')===curTag?'true':'false'));
      runFilter();
    }}));
    addEventListener('keydown',e=>{{
      if(e.key==='/'&&!e.metaKey&&!e.ctrlKey&&!e.altKey&&!/^(INPUT|TEXTAREA|SELECT)$/.test((e.target&&e.target.tagName)||'')){{
        e.preventDefault();inp.focus();
      }}
    }});
  }}
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


def rebuild_all(cfg: dict[str, str]) -> list[Issue]:
    """Build plaintext (nav already injected) under site/plain/.

    Does not write issue HTML into public/; encrypt_all_pages does that.
    Root files (robots.txt, 404, _headers, _redirects) are written unencrypted to public/.
    """
    slug = cfg["SLUG"]
    issues = list_sources()
    write_static_root(
        slug=slug,
        issues=issues,
        root_latest=_parse_bool(cfg.get("ROOT_LATEST")),
    )
    plain_dir = PLAIN / slug
    _clear_files(plain_dir)
    # public/<slug> is NOT cleared: unchanged encrypted pages are reused
    # (see encrypt_all_pages cache); stale ones are pruned after encryption.
    (plain_dir / "daily").mkdir(parents=True, exist_ok=True)
    (plain_dir / "weekly").mkdir(parents=True, exist_ok=True)

    for issue in issues:
        html = issue.source.read_text(encoding="utf-8")
        html = strip_injections(html)
        html = ensure_noindex(html)
        html = ensure_nav_css(html)
        html = inject_nav(html, issue, issues)
        html = inject_perf_and_lock(html, issue, issues, slug=slug)
        html = minify_css(html)
        dest = plain_dir / issue.kind / f"{issue.label}.html"
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(html, encoding="utf-8")

    index_path = plain_dir / "index.html"
    index_path.write_text(minify_css(index_html(cfg, issues)), encoding="utf-8")
    return issues


def privacy_scan(paths: Iterable[Path]) -> None:
    hits: list[str] = []
    compiled = [(re.compile(pat, re.I | re.S), name) for pat, name in PRIVACY_PATTERNS + _load_extra_privacy_patterns()]
    for path in paths:
        if not path.is_file():
            continue
        # Skip binary just in case
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        for rx, name in compiled:
            for m in rx.finditer(text):
                snippet = m.group(0).replace("\n", " ")
                if len(snippet) > 80:
                    snippet = snippet[:80] + "…"
                hits.append(f"{path}: [{name}] {snippet}")
    if hits:
        print("privacy scan FAILED:", file=sys.stderr)
        for h in hits:
            print(f"  {h}", file=sys.stderr)
        die("refusing to publish; remove personal info and retry")
    print("privacy scan: clean", file=sys.stderr)


def iter_public_files() -> list[Path]:
    files = [p for p in PUBLIC.rglob("*") if p.is_file()]
    files = [p for p in files if p.name != ".DS_Store"]
    return sorted(files)


def iter_plain_files() -> list[Path]:
    if not PLAIN.exists():
        return []
    files = [p for p in PLAIN.rglob("*") if p.is_file() and p.name != ".DS_Store"]
    return sorted(files)


def iter_unencrypted_public() -> list[Path]:
    """Root files that stay in plaintext: robots.txt, 404.html, _headers, _redirects."""
    out: list[Path] = []
    for name in ("robots.txt", "_headers", "404.html", "_redirects"):
        p = PUBLIC / name
        if p.is_file():
            out.append(p)
    return out


def pages_hash(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()[:32]


def content_type_for(path: Path, rel: str) -> str:
    name = path.name
    if name == "_headers" or name == "_redirects":
        return "text/plain; charset=utf-8"
    if name.endswith(".html"):
        return "text/html; charset=utf-8"
    if name.endswith(".txt"):
        return "text/plain; charset=utf-8"
    guessed, _ = mimetypes.guess_type(str(path))
    return guessed or "application/octet-stream"


def build_manifest() -> tuple[dict[str, str], dict[str, dict]]:
    """Return (manifest path->hash, files path->{hash,content,contentType})."""
    manifest: dict[str, str] = {}
    files: dict[str, dict] = {}
    for path in iter_public_files():
        rel = path.relative_to(PUBLIC).as_posix()
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


def public_urls(cfg: dict[str, str], issues: list[Issue]) -> list[tuple[str, str]]:
    base = f"https://{cfg['DOMAIN']}/{cfg['SLUG']}"
    out = [("index", f"{base}/")]
    for i in sorted(issues, key=lambda x: (x.kind, x.start, x.label)):
        out.append((f"{i.kind}/{i.label}", f"{base}/{i.kind}/{i.label}"))
    return out



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


_PASSWORD_OVERRIDE: str | None = None  # demo mode only (never persisted)
_SALT_OVERRIDE: bytes | None = None


def read_password() -> str:
    if _PASSWORD_OVERRIDE is not None:
        return _PASSWORD_OVERRIDE
    path = password_file_path()
    if not path.is_file():
        die(
            f"password file not found: {path}. "
            "Create it (20+ chars) or run --rotate-password."
        )
    mode = path.stat().st_mode
    if mode & (stat.S_IRGRP | stat.S_IROTH | stat.S_IWGRP | stat.S_IWOTH):
        print(f"warning: {path} should be mode 600", file=sys.stderr)
    # Strip a single surrounding newline only; do not log the value.
    pw = path.read_bytes().decode("utf-8")
    if pw.endswith("\n"):
        pw = pw[:-1]
    if pw.endswith("\r"):
        pw = pw[:-1]
    if len(pw) < 16:
        die("password file is too short")
    return pw


def load_or_create_salt() -> bytes:
    if _SALT_OVERRIDE is not None:
        return _SALT_OVERRIDE
    path = salt_file_path()
    if path.is_file():
        raw = path.read_text(encoding="ascii").strip()
        try:
            salt = bytes.fromhex(raw)
        except ValueError:
            die(f"salt file {path} is not valid hex")
        if len(salt) < 16:
            die("salt file is too short")
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


def localize_template(tmpl: str, lang: str | None = None, extras: dict[str, str] | None = None) -> str:
    """Fill {{key}} (HTML-escaped) and {{js:key}} (JSON string literal) from i18n.STRINGS.

    `extras` supplies pre-built HTML for keys such as {{lang_switch}}; a key that is not
    in `extras` and not in the string table is an error, except lang_switch and demo_hint (empty).
    """
    lang = lang or LANG
    extras = extras or {}

    def sub(m: re.Match) -> str:
        js, key = m.group(1) == "js:", m.group(2)
        if key in extras:
            return extras[key]
        if key in ("lang_switch", "demo_hint"):
            return ""
        try:
            val = i18n.t(lang, key)
        except KeyError:
            die(f"password template uses unknown i18n key {key!r}")
        return json.dumps(val, ensure_ascii=False) if js else _esc(val)

    return re.sub(r"\{\{(js:)?(\w+)\}\}", sub, tmpl)


def shell_extras(rel: str = "") -> dict[str, str]:
    """Extra HTML for the password shell of the page at `rel` (e.g. daily/2026-01-15.html)."""
    extras = {"lang_switch": lang_switch_link(rel, "lang-alt")}
    if _PASSWORD_OVERRIDE is not None:  # demo builds only: the demo password is public by design
        extras["demo_hint"] = (
            '<p class="demo-hint">'
            + _esc(i18n.t(LANG, "pw_demo_hint")).replace("{pw}", f"<code>{_esc(_PASSWORD_OVERRIDE)}</code>")
            + "</p>"
        )
    return extras


def wrap_encrypted(plaintext: str, key: bytes, salt: bytes, tmpl: str, rel: str = "") -> str:
    nonce = os.urandom(NONCE_LEN)
    # v2: gzip before encrypting (ciphertext is incompressible, so the CDN's
    # brotli cannot help); the page inflates with DecompressionStream.
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
        die("password template is missing the __ENCRYPT_CFG_JSON__ placeholder")
    tmpl = localize_template(tmpl, extras=shell_extras(rel))
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
    # Dedup, keep order
    seen: set[str] = set()
    out: list[str] = []
    for s in snippets:
        if s not in seen:
            seen.add(s)
            out.append(s)
    return out[:30]


def _assert_no_plaintext_leak(plain: str, wrapped: str, rel: Path, tmpl: str) -> None:
    tmpl = localize_template(tmpl, extras=shell_extras(rel.as_posix()))  # the shell's own (localized) words are not a leak
    for snip in _distinctive_snippets(plain, tmpl):
        if snip in wrapped:
            die(f"plaintext leak in encrypted {rel}: {snip!r}")
    # Prompt page must be present
    for needle in (_t("pw_title"), _t("pw_label"), _t("pw_open"), PROMPT_MARK, "noindex"):
        if needle not in wrapped:
            die(f"encrypted {rel} is missing the password prompt ({needle})")
    if "__ENCRYPT_CFG_JSON__" in wrapped:
        die(f"encrypted {rel} still has an unreplaced template placeholder")
    if '"ct"' not in wrapped and '"ct":' not in wrapped:
        # json.dumps uses "ct":
        die(f"encrypted {rel} is missing ciphertext")


def encrypt_all_pages(cfg: dict[str, str]) -> None:
    """Encrypt every HTML page under site/plain/<slug>/ into public/<slug>/."""
    if not TEMPLATE_PATH.is_file():
        die(f"password template not found: {TEMPLATE_PATH}")
    tmpl = TEMPLATE_PATH.read_text(encoding="utf-8")
    password = read_password()
    salt = load_or_create_salt()
    key = derive_key(password, salt)
    # Drop the password string as soon as the key is derived.
    password = None  # noqa: F841

    slug = cfg["SLUG"]
    src_root = PLAIN / slug
    dst_root = PUBLIC / slug
    if not src_root.is_dir():
        die("plaintext pages not found under site/plain; run a rebuild first")
    html_files = sorted(src_root.rglob("*.html"))
    if not html_files:
        die("no plaintext HTML to encrypt")

    # Content-hash cache: a page is re-encrypted only when its plaintext,
    # the derived key/salt, the template, or the shell language changed.
    # Encryption uses a random IV, so re-encrypting unchanged pages would
    # change ciphertext and force re-upload; we keep the prior encrypted file.
    cache = _load_enc_cache()
    salt_fp = hashlib.sha256(salt).hexdigest()[:16]
    key_fp = hashlib.sha256(key).hexdigest()[:16]
    tmpl_fp = hashlib.sha256(tmpl.encode("utf-8")).hexdigest()[:16]
    new_cache: dict[str, str] = {}
    n = skipped = 0
    CHANGED_PAGES.clear()
    for src in html_files:
        rel = src.relative_to(src_root)
        plain = src.read_text(encoding="utf-8")
        alt_fp = hashlib.sha256((lang_switch_href(rel.as_posix()) or "").encode()).hexdigest()[:8]
        fp = f"{hashlib.sha256(plain.encode('utf-8')).hexdigest()}:{salt_fp}:{key_fp}:{tmpl_fp}:{alt_fp}:{LANG}"
        dest = dst_root / rel
        key_rel = rel.as_posix()
        if cache.get(key_rel) == fp and dest.is_file():
            body = dest.read_text(encoding="utf-8")
            if PROMPT_MARK in body and '"ct":' in body:
                new_cache[key_rel] = fp
                skipped += 1
                continue
        wrapped = wrap_encrypted(plain, key, salt, tmpl, key_rel)
        _assert_no_plaintext_leak(plain, wrapped, rel, tmpl)
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(wrapped, encoding="utf-8")
        new_cache[key_rel] = fp
        CHANGED_PAGES.append(key_rel)
        n += 1
    # Prune encrypted pages whose plaintext no longer exists (removed issues).
    keep = {s.relative_to(src_root).as_posix() for s in html_files}
    for pub in sorted(dst_root.rglob("*.html")):
        r = pub.relative_to(dst_root).as_posix()
        if r not in keep:
            pub.unlink()
            CHANGED_PAGES.append(f"-{r}")
    _save_enc_cache(new_cache)
    print(
        f"encrypted {n} HTML page(s), reused {skipped} unchanged, with AES-256-GCM / "
        f"PBKDF2-SHA256 ({PBKDF2_ITERATIONS} iterations)",
        file=sys.stderr,
    )


ENC_CACHE_PATH = SITE / "enc-cache.json"
CHANGED_PAGES: list[str] = []


def _load_enc_cache() -> dict[str, str]:
    try:
        return json.loads(ENC_CACHE_PATH.read_text(encoding="utf-8"))
    except Exception:
        return {}


def _save_enc_cache(cache: dict[str, str]) -> None:
    ENC_CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
    tmp = ENC_CACHE_PATH.with_name(ENC_CACHE_PATH.name + ".tmp")
    tmp.write_text(json.dumps(cache, indent=1, sort_keys=True), encoding="utf-8")
    os.replace(tmp, ENC_CACHE_PATH)


def assert_public_is_encrypted(cfg: dict[str, str]) -> None:
    """Refuse to deploy if issue/index HTML under the slug is still plaintext."""
    slug_dir = PUBLIC / cfg["SLUG"]
    pages = sorted(slug_dir.rglob("*.html")) if slug_dir.exists() else []
    if not pages:
        die("no published HTML under public/<slug>; run a rebuild first")
    tmpl = TEMPLATE_PATH.read_text(encoding="utf-8") if TEMPLATE_PATH.is_file() else ""
    for path in pages:
        text = path.read_text(encoding="utf-8")
        if PROMPT_MARK not in text or _t("pw_title") not in text:
            die(f"{path.relative_to(PUBLIC)} does not look encrypted; run --rebuild")
        if "__ENCRYPT_CFG_JSON__" in text:
            die(f"{path.relative_to(PUBLIC)} has an unreplaced placeholder")
        # If we still have plaintext copies, confirm they did not leak.
        rel = path.relative_to(slug_dir)
        plain_path = (PLAIN / cfg["SLUG"] / rel)
        if plain_path.is_file():
            _assert_no_plaintext_leak(
                plain_path.read_text(encoding="utf-8"), text, rel, tmpl
            )
    # Root files must remain unencrypted and must not be prompt pages.
    for name in ("robots.txt", "_headers", "404.html"):
        path = PUBLIC / name
        if not path.is_file():
            die(f"missing unencrypted root file: {name}")
        body = path.read_text(encoding="utf-8")
        if PROMPT_MARK in body or "__ENCRYPT_CFG_JSON__" in body:
            die(f"{name} must stay unencrypted")
    redirects_path = PUBLIC / "_redirects"
    if redirects_path.is_file():
        body = redirects_path.read_text(encoding="utf-8")
        if PROMPT_MARK in body or "__ENCRYPT_CFG_JSON__" in body:
            die("_redirects must stay unencrypted")
    print(f"encryption check: {len(pages)} page(s) look encrypted", file=sys.stderr)


def print_urls(cfg: dict[str, str], issues: list[Issue]) -> None:
    for name, url in public_urls(cfg, issues):
        print(f"{name}: {url}")


def prepare(html_path: Path | None, kind: str | None, label: str | None) -> tuple[dict[str, str], list[Issue]]:
    cfg = init_config_if_needed()
    if html_path is not None:
        if not html_path.is_file():
            die(f"HTML file not found: {html_path}")
        if kind not in ("daily", "weekly"):
            die("type must be daily or weekly")
        if kind == "daily":
            parse_daily_label(label)  # type: ignore[arg-type]
        else:
            parse_weekly_label(label)  # type: ignore[arg-type]
        save_source(html_path, kind, label)  # type: ignore[arg-type]
        print(f"saved source {kind}/{label}", file=sys.stderr)
    issues = rebuild_all(cfg)
    # Privacy scan MUST run on plaintext, before encryption. Ciphertext is
    # base64 and can false-positive the email regex.
    privacy_scan(iter_plain_files() + iter_unencrypted_public())
    encrypt_all_pages(cfg)
    assert_public_is_encrypted(cfg)
    print(f"prepared {len(issues)} issue(s); index at /{cfg['SLUG']}/", file=sys.stderr)
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

    Skips (loudly) any issue whose markdown is missing; its old source stays.
    Normal runs never call this.
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
        render_html.render_file(md, out, LANG, year=issue.start.year, en_md_path=en_md)
        save_source(out, issue.kind, issue.label)
        extra = f" + {en_md.name}" if en_md else ""
        print(f"rerender: {issue.kind}/{issue.label} <- {md.name}{extra}", file=sys.stderr)


# ---------------------------------------------------------------------------
# Demo build (CI-friendly): examples/*.md -> encrypted static site in --out DIR.
# Non-interactive, no Cloudflare, nothing written outside the temp workdir and
# the output dir. The password comes from env DIGEST_DEMO_PASSWORD (a *public*
# demo password; never reuse a real one). The salt is random per build.
# ---------------------------------------------------------------------------

DEMO_DAILY_LABEL = "2026-01-15"
DEMO_WEEKLY_LABEL = "2026-01-12_01-18"


def _verify_demo_pages(out_slug_dir: Path, plain_slug_dir: Path, key: bytes) -> int:
    """Decrypt every output page with the demo key and compare to its plaintext."""
    n = 0
    for page in sorted(out_slug_dir.rglob("*.html")):
        text = page.read_text(encoding="utf-8")
        i = text.index('id="c">') + len('id="c">')
        cfg = json.loads(text[i:text.index("</script>", i)])
        packed = AESGCM(key).decrypt(
            base64.b64decode(cfg["iv"]), base64.b64decode(cfg["ct"]), None
        )
        plain = gzip.decompress(packed).decode("utf-8") if cfg.get("z") == "gzip" else packed.decode("utf-8")
        expect = (plain_slug_dir / page.relative_to(out_slug_dir)).read_text(encoding="utf-8")
        if plain != expect:
            die(f"demo verify failed: {page} decrypts to different plaintext")
        n += 1
    return n


DEMO_LANGS = ("en", "zh-CN")


def build_demo(out: Path, examples: Path, daily_label: str, weekly_label: str,
               langs: tuple[str, ...] | None = None) -> int:
    """Build the demo site from examples/ into `out` (no deploy).

    `langs=None` or one language: a single site at /<slug>/ (slug "demo" by default).
    Both languages (DEMO_LANGS): English at /en/ and Chinese at /zh-CN/ in one output tree,
    sharing one salt (unlocking one language unlocks the other), with a language-switch link
    on every page and `/` redirecting to /en/.
    """
    global SITE, PUBLIC, PLAIN, SOURCES, CONFIG_PATH, ENC_CACHE_PATH
    global _PASSWORD_OVERRIDE, _SALT_OVERRIDE, CLI_LANG, LANG_ALT, LANG, _DEMO_MODE
    import shutil
    import tempfile

    password = os.environ.get("DIGEST_DEMO_PASSWORD", "")
    if len(password) < 4:
        die("set env DIGEST_DEMO_PASSWORD (>= 4 chars; it is a public demo password) for --demo")
    langs = tuple(langs) if langs else (LANG,)
    multi = len(langs) > 1
    daily_date = parse_daily_label(daily_label)
    weekly_start, _ = parse_weekly_label(weekly_label)
    for lg in langs:
        if not (examples / f"sample-digest.{lg}.md").is_file():
            die(f"missing {examples / f'sample-digest.{lg}.md'}")

    saved_paths = (SITE, PUBLIC, PLAIN, SOURCES, CONFIG_PATH, ENC_CACHE_PATH)
    saved_overrides = (_PASSWORD_OVERRIDE, _SALT_OVERRIDE, CLI_LANG, LANG_ALT, LANG, _DEMO_MODE)
    saved_slug = os.environ.get("SITE_SLUG")
    saved_lang_alt = os.environ.get("DIGEST_LANG_ALT")
    saved_root_latest = os.environ.get("DIGEST_ROOT_LATEST")
    os.environ.pop("DIGEST_LANG_ALT", None)
    os.environ.pop("DIGEST_ROOT_LATEST", None)
    if multi:  # the language prefixes are the slugs; an env slug would override them in load_config()
        os.environ.pop("SITE_SLUG", None)
    work = Path(tempfile.mkdtemp(prefix="ai-digest-demo-"))
    try:
        _DEMO_MODE = True
        SITE = work / "site"
        PUBLIC, PLAIN = SITE / "public", SITE / "plain"
        CONFIG_PATH = SITE / "CONFIG"
        _PASSWORD_OVERRIDE = password
        _SALT_OVERRIDE = os.urandom(SALT_LEN)
        key = derive_key(password, _SALT_OVERRIDE)
        if str(CODE_ROOT) not in sys.path:
            sys.path.insert(0, str(CODE_ROOT))
        import render_html

        slugs = {lg: (lg if multi else (saved_slug or "demo")) for lg in langs}
        total_issues = total_pages = 0
        demo_latest_rules: list[str] = []
        for lg in langs:
            CLI_LANG = lg  # beats $DIGEST_LANG inside load_config()
            set_lang(lg)
            other = next((o for o in langs if o != lg), None)
            LANG_ALT = {"code": other, "base": f"/{slugs[other]}/"} if multi and other else None
            SOURCES = SITE / f"sources-{lg}"
            ENC_CACHE_PATH = SITE / f"enc-cache-{lg}.json"
            cfg_data = {
                "PLATFORM": PLATFORM,
                "PROJECT": os.environ.get("CF_PAGES_PROJECT") or PROJECT_DEFAULT,
                "ACCOUNT_ID": ACCOUNT_ID_DEFAULT,
                "DOMAIN": os.environ.get("SITE_DOMAIN") or DOMAIN_DEFAULT,
                "SLUG": slugs[lg],
                "LANG": lg,
            }
            if multi and other:
                cfg_data["LANG_ALT"] = f"{other}:/{slugs[other]}/"
            write_config(cfg_data)
            html_dir = work / f"html-{lg}"
            sample_daily = examples / f"sample-digest.{lg}.md"
            sample_weekly = examples / f"sample-weekly.{lg}.md"
            jobs = [(sample_daily, "daily", daily_label, daily_date.year)]
            if sample_weekly.is_file():
                jobs.append((sample_weekly, "weekly", weekly_label, weekly_start.year))
            for md, kind, label, issue_year in jobs:
                h = html_dir / f"{kind}-{label}.html"
                render_html.render_file(md, h, lg, year=issue_year)
                save_source(h, kind, label)
            cfg = load_config()
            issues = rebuild_all(cfg)
            demo_latest_rules.extend(_latest_redirect_rules(slugs[lg], issues))
            privacy_scan(iter_plain_files() + iter_unencrypted_public())
            encrypt_all_pages(cfg)
            assert_public_is_encrypted(cfg)
            total_pages += _verify_demo_pages(PUBLIC / slugs[lg], PLAIN / slugs[lg], key)
            total_issues += len(issues)
        entry = slugs[langs[0]]
        # Pages: send the site root to the (first language's) index plus per-language latest aliases; the demo has no Worker.
        demo_redirects = [f"/ /{entry}/ 302", *demo_latest_rules]
        (PUBLIC / "_redirects").write_text("\n".join(demo_redirects) + "\n", encoding="utf-8")
        if multi:  # one _headers with cache rules for every language prefix
            rules = "".join(_slug_cache_headers(sl) for sl in slugs.values())
            (PUBLIC / "_headers").write_text(SECURITY_HEADERS + rules, encoding="utf-8")
        if out.exists():
            shutil.rmtree(out)
        shutil.copytree(PUBLIC, out)
        where = ", ".join(f"/{sl}/" for sl in slugs.values())
        print(f"demo site: {total_issues} issue(s), {total_pages} page(s) verified by decrypt -> {out}/ (entry: /{entry}/; {where})")
        return 0
    finally:
        SITE, PUBLIC, PLAIN, SOURCES, CONFIG_PATH, ENC_CACHE_PATH = saved_paths
        _PASSWORD_OVERRIDE, _SALT_OVERRIDE, CLI_LANG, LANG_ALT, LANG, _DEMO_MODE = saved_overrides
        if saved_slug is not None:
            os.environ["SITE_SLUG"] = saved_slug
        if saved_lang_alt is not None:
            os.environ["DIGEST_LANG_ALT"] = saved_lang_alt
        if saved_root_latest is not None:
            os.environ["DIGEST_ROOT_LATEST"] = saved_root_latest
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
    parser.add_argument("--rebuild", action="store_true", help="rebuild nav, index, and static files from sources, then encrypt")
    parser.add_argument("--prepare-only", action="store_true", help="do not deploy")
    parser.add_argument("--deploy-only", action="store_true", help="deploy already-prepared public/ tree")
    parser.add_argument("--init", action="store_true", help="create CONFIG and exit")
    parser.add_argument(
        "--rotate-password",
        action="store_true",
        help=(
            "generate a new password (same file, mode 600) and a new salt, "
            "re-encrypt all issues from sources, and deploy (unless --prepare-only). "
            "Never prints the password."
        ),
    )
    parser.add_argument("--rerender-all", action="store_true", help="re-render every issue in site/sources from its markdown (digest-LABEL.md / weekly-LABEL.md) with the current render_html.py, then rebuild + deploy (use after a template change; implies --rebuild)")
    parser.add_argument("--lang", help="output language: en | zh-CN (default: $DIGEST_LANG, else CONFIG LANG, else en). With --demo also: all (English at /en/ + Chinese at /zh-CN/; the --demo default when no language is given)")
    parser.add_argument("--demo", action="store_true", help="build the demo site from examples/ into --out (no deploy, no config/secrets needed; password from env DIGEST_DEMO_PASSWORD)")
    parser.add_argument("--out", default="dist", help="with --demo: output directory (default: dist)")
    parser.add_argument("--examples", default=None, help="with --demo: examples directory (default: <repo>/examples)")
    parser.add_argument("--issue", help="incremental publish of one issue: daily/YYYY-MM-DD or weekly/YYYY-MM-DD_MM-DD")
    parser.add_argument("--md", help="with --issue: markdown to render (render_html.py) before publishing")
    parser.add_argument("--md-en", help="with --issue --md: English companion markdown for single-page bilingual rendering (auto-discovered as <stem>.en.md when LANG=zh-CN)")
    parser.add_argument("--remove", help="remove one issue (daily/LABEL or weekly/LABEL); backup to backfill/removed/")
    parser.add_argument("--no-verify", action="store_true", help="with --issue/--remove: skip post-deploy decrypt check")
    parser.add_argument("--verbose", action="store_true", help="with --issue/--remove: stream full logs instead of a short summary")
    args = parser.parse_args(argv)

    global CLI_LANG
    demo_all = bool(args.demo and (args.lang == "all" or (not args.lang and not os.environ.get("DIGEST_LANG", "").strip())))
    CLI_LANG = None if args.lang == "all" else args.lang
    set_lang(CLI_LANG)

    if args.init:
        cfg = init_config_if_needed()
        print(f"SLUG={cfg['SLUG']}")
        return 0

    if args.demo:
        set_lang(CLI_LANG)  # demo ignores site/CONFIG: --lang > $DIGEST_LANG > both languages
        return build_demo(
            Path(args.out),
            Path(args.examples) if args.examples else CODE_ROOT / "examples",
            DEMO_DAILY_LABEL, DEMO_WEEKLY_LABEL,
            DEMO_LANGS if demo_all else None,
        )

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
        rotate_password_and_salt()
        ENC_CACHE_PATH.unlink(missing_ok=True)  # full re-encrypt
        # Re-encrypt from sources (and save a new issue if one was given).
        html_path = Path(args.html) if args.html else None
        cfg, issues = prepare(html_path, args.kind, args.date_label)
    elif args.deploy_only:
        cfg = load_config()
        issues = list_sources()
        if not issues and not (PUBLIC / cfg["SLUG"] / "index.html").exists():
            die("nothing to deploy; publish an issue first")
        # Never privacy-scan ciphertext (base64 can false-positive).
        scan_paths = iter_unencrypted_public() + iter_plain_files()
        if scan_paths:
            privacy_scan(scan_paths)
        assert_public_is_encrypted(cfg)
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
# nav/index from site/sources (cheap, deterministic), re-encrypts only pages
# whose plaintext changed (enc-cache.json), deploys (Wrangler uploads only
# new hashes), checks HTTP + decrypt, and prints a few summary lines.
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
            base = f"https://{cfg['DOMAIN']}/{cfg['SLUG']}"
            issue_url = f"{base}/{kind}/{label}"
            items_note = ""
            if args.remove:
                src = SOURCES / kind / f"{label}.html"
                bdir = REMOVED_DIR / f"{kind}-{label}-{stamp}"
                bdir.mkdir(parents=True, exist_ok=True)
                moved = []
                for p in (src, ROOT / "html" / f"{kind}-{label}.html"):
                    if p.is_file():
                        p.rename(bdir / p.name if p != src else bdir / f"source-{p.name}")
                        moved.append(p.name)
                for sub in ("plain", "public"):
                    p = SITE / sub / cfg["SLUG"] / kind / f"{label}.html"
                    if p.is_file():
                        if sub == "public":
                            (bdir / f"encrypted-{p.name}").write_bytes(p.read_bytes())
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
                elif not html_path.is_file():
                    die(f"no --md given and {html_path} does not exist")
                rf = SITE / "removed-paths.txt"
                if rf.is_file():
                    keep = [x for x in rf.read_text(encoding="utf-8").split() if x != f"{kind}/{label}"]
                    rf.write_text("\n".join(keep) + ("\n" if keep else ""), encoding="utf-8")
                cfg, issues = prepare(html_path, kind, label)
                summary.append(f"published: {kind}/{label}{items_note}")
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
                    summary.append(f"decrypt check: {vmsg}")
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
