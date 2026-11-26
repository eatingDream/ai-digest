#!/usr/bin/env python3
"""Pure shared helpers for fetch/ scripts (zero dependency on fetch.py)."""
from __future__ import annotations

import json
import os
import re
import tempfile
from pathlib import Path
from urllib.parse import parse_qsl, urlencode, urlparse, urlunparse
from zoneinfo import ZoneInfo

FETCH_DIR = Path(__file__).resolve().parent
CODE_ROOT = FETCH_DIR.parent

ARXIV_ID_RE = re.compile(r"(?<!\d)(\d{4}\.\d{4,5})(?!\d)")
ARXIV_URL_RE = re.compile(r"arxiv\.org/(?:abs|pdf|html)/(\d{4}\.\d{4,5})")
HF_PAPER_RE = re.compile(r"huggingface\.co/papers/(\d{4}\.\d{4,5})")

TRACK_PARAMS = frozenset({
    "utm_source", "utm_medium", "utm_campaign", "utm_content", "utm_term",
    "ref", "ref_src", "source", "rss", "src", "fbclid", "gclid",
    "mc_cid", "mc_eid", "smid", "cmpid", "sr_share",
})


def get_tz_local() -> ZoneInfo:
    """Resolve local timezone lazily from $DIGEST_TZ (default UTC)."""
    return ZoneInfo(os.environ.get("DIGEST_TZ") or "UTC")


def work_root(code_root: Path = CODE_ROOT) -> Path:
    env = os.environ.get("DIGEST_ROOT", "").strip()
    return Path(env) if env else code_root


def state_dir(code_root: Path = CODE_ROOT, fetch_dir: Path = FETCH_DIR) -> Path:
    env = os.environ.get("DIGEST_ROOT", "").strip()
    return (Path(env) / "state") if env else (fetch_dir / "state")


def ledger_path(code_root: Path = CODE_ROOT) -> Path:
    env = os.environ.get("LEDGER_PATH", "").strip()
    return Path(env) if env else (work_root(code_root) / "ledger.json")


def canon_url(u: str) -> str:
    """Canonicalize URLs, collapsing arXiv/HF paper URLs and stripping tracking params."""
    u = (u or "").strip()
    if not u:
        return u
    m = ARXIV_URL_RE.search(u)
    if m:
        return f"arxiv:{m.group(1)}"
    m = HF_PAPER_RE.search(u)
    if m:
        return f"arxiv:{m.group(1)}"
    p = urlparse(u)
    q = [(k, v) for k, v in parse_qsl(p.query) if k.lower() not in TRACK_PARAMS and not k.lower().startswith("utm_")]
    host = p.netloc.lower().removeprefix("www.").removeprefix("m.")
    path = re.sub(r"/+$", "", p.path) or "/"
    return urlunparse(("https", host, path, "", urlencode(q), ""))


def norm_title(t: str) -> str:
    """Normalize a title to lowercase alphanumerics/CJK (max 80 chars)."""
    t = t.lower()
    t = re.sub(r"[^\w\u4e00-\u9fff]+", "", t)
    return t[:80]


def ledger_status(it: dict) -> str:
    """Classify a ledger item as 'seen' or 'pushed'."""
    s = f"{it.get('status') or ''} {it.get('category') or ''}"
    return "seen" if "seen" in s else "pushed"


def load_ledger_keys(path: Path | None = None) -> dict[str, str]:
    """Read ledger.json into {canon_key: 'pushed' | 'seen'}."""
    keys: dict[str, str] = {}
    p = Path(path) if path is not None else ledger_path()
    try:
        d = json.loads(p.read_text(encoding="utf-8"))
        for it in d.get("items", []):
            st = ledger_status(it)
            ks = [canon_url(u) for u in it.get("urls", []) or []]
            if it.get("id") and re.match(r"^\d{4}\.\d{4,5}$", str(it["id"])):
                ks.append(f"arxiv:{it['id']}")
            if it.get("name"):
                ks.append("t:" + norm_title(it["name"]))
            for k in ks:
                if keys.get(k) != "pushed":
                    keys[k] = st
    except Exception:
        pass
    return keys


def atomic_write_text(path: Path | str, text: str, encoding: str = "utf-8") -> None:
    """Write text to a temp file in the target directory and atomically replace target."""
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=f".{p.name}.", suffix=".tmp", dir=str(p.parent))
    try:
        with os.fdopen(fd, "w", encoding=encoding) as fh:
            fh.write(text)
            fh.flush()
        os.replace(tmp, p)
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
