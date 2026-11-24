#!/usr/bin/env python3
"""Recall layer: fetch every source in sources.toml in parallel and write a de-duplicated candidates.jsonl.

Natural-day windows in local time ($DIGEST_TZ, default UTC): run just after midnight, produce yesterday's issue.
  python3 fetch/fetch.py --daily                      # P = yesterday; window = P 00:00 -> P+1 00:00, half-open [start,end)
  python3 fetch/fetch.py --daily --day 2026-01-15     # a specific P
  python3 fetch/fetch.py --daily --day 2026-01-15 --start 2026-01-15T09:00   # override the start only
  python3 fetch/fetch.py --weekly                     # last Monday 00:00 -> this Monday 00:00; --day X = the week containing X
  python3 fetch/fetch.py --weekly --start 2026-01-11T21:00   # override the start (e.g. the first weekly)
  python3 fetch/fetch.py --start 2026-01-14T09:00 --end 2026-01-15T09:00 [--out DIR]   # any window
  python3 fetch/fetch.py --daily --json               # also print one JSON summary object on stdout
Common options: --only arxiv,hf_daily  --skip hn  --no-state (do not read/write incremental state)
  --catchup (also collect earlier missed arXiv batches; off by default)  --lang en|zh-CN (labels in triage/report)
Output (default <repo>/runs/<P>[-weekly]/fetch/; a weekly is named after the week's Sunday):
  candidates.jsonl   one de-duplicated candidate per line
  triage.md          the list sorted by channel/topic for a quick skim
  report.json/.md    per-source count, time and errors
Progress and the warnings (failed sources, "Note:" lines such as a missing arXiv batch) go to stderr;
--json adds a summary to stdout: {"window", "sources", "raw", "dedup", "errors", "out", "files"}.
Exit codes: 0 finished (individual source errors are listed, not fatal); 2 bad command line.
Needs only the standard library + requests + lxml.
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import html as htmlmod
import json
import os
import re
import sys
import threading
import time
import tomllib
import traceback
from dataclasses import dataclass, field, asdict
from datetime import datetime, timedelta, timezone
from email.utils import parsedate_to_datetime
from pathlib import Path
from urllib.parse import urljoin, urlparse, urlunparse, parse_qsl, urlencode
from zoneinfo import ZoneInfo

import requests
from lxml import etree, html as lhtml

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))
import i18n  # noqa: E402  (shared UI/label strings, see i18n.py)
from version import __version__  # noqa: E402

# Output language of triage/report labels: --lang > $DIGEST_LANG > en.
FL = i18n.resolve_lang()


def tr(key: str, **kw) -> str:
    return i18n.ft(FL, key, **kw)


# 本地时区（日界线/窗口所用）：环境变量 DIGEST_TZ（IANA 名，如 Asia/Shanghai），默认 UTC。
TZ_LOCAL = ZoneInfo(os.environ.get("DIGEST_TZ") or "UTC")
ET = ZoneInfo("America/New_York")
UTC = timezone.utc

# ───────────────────────── 关键词：AI 过滤 + 主题标签 + 偏好权重 ─────────────────────────
AI_RE = re.compile(
    r"(?i)\b(a\.?i\.?|ai's|artificial intelligence|machine learning|deep learning|neural|llms?|gpt|chatgpt|"
    r"openai|anthropic|claude|gemini|deepmind|grok|xai|mistral|llama|qwen|deepseek|kimi|moonshot|zhipu|glm|minimax|"
    r"copilot|cursor|agentic|agents?|transformer|diffusion|inference|gpu|gpus|nvidia|tpu|cuda|h100|h200|b200|b300|gb200|gb300|"
    r"blackwell|rubin|hbm|datacenter|data center|chatbot|reasoning model|frontier model|foundation model|"
    r"robot|robotics|humanoid|embodied|hugging ?face|alignment|interpretability|superintelligence|agi|rlhf|"
    r"fine-?tun\w*|pretrain\w*|token|benchmark|multimodal|text-to-\w+|world model|autonomous)\b"
    r"|人工智能|大模型|智能体|模型|算力|芯片|机器人|具身|生成式|算法|AI|英伟达|推理|训练|开源|多模态|自动驾驶"
)

TOPICS: list[tuple[str, str]] = [
    ("harness", r"harness|scaffold|agent framework|agent runtime|coding agent|computer[- ]use|tool[- ]use|tool calling|mcp\b|model context protocol|claude code|codex|skills?\b|workflow"),
    ("rsi", r"self[- ]improv|self[- ]evolv|recursive|rsi\b|automated (ai )?research|ai scientist|self[- ]play|self[- ]refin|meta[- ]learn|open[- ]ended|auto(mated)? ?ml"),
    ("training", r"reinforcement learning|\brl\b|rlvr|rlhf|grpo|ppo\b|dpo\b|reward|post[- ]training|pre[- ]?training|distill|fine[- ]?tun|sft\b|curriculum|scaling law|optimizer|data mixture|synthetic data|on[- ]policy|mixture[- ]of[- ]experts|\bmoe\b"),
    ("inference", r"inference|serving|speculative|kv[- ]?cache|quantiz|decoding|latency|throughput|vllm|sglang|llama\.cpp|long[- ]context|attention kernel"),
    ("agents", r"\bagents?\b|agentic|multi[- ]agent|planning|memory|browser|web agent|swe-bench|terminal-bench|tau.?bench"),
    ("reasoning", r"reasoning|chain[- ]of[- ]thought|\bcot\b|math|theorem|proof|test[- ]time"),
    ("multimodal", r"vision|image|video|audio|speech|tts\b|asr\b|multimodal|vlm|text-to-|diffusion|3d"),
    ("robotics", r"robot|embodied|humanoid|manipulation|locomotion|vla\b|autonomous driving|self-driving"),
    ("safety", r"safety|alignment|interpretab|mechanistic|jailbreak|red[- ]team|deception|sycophan|misalign|monitor|sabotage|honest|reward hacking|control\b|bio(risk|security)|cyber"),
    ("evals", r"benchmark|evaluation|\beval|leaderboard|arena"),
    ("data", r"dataset|corpus|data curation|data quality|benchmark data|annotation"),
    ("infra", r"gpu|tpu|cluster|datacenter|data center|chip|semiconductor|hbm|nvidia|amd|tsmc|compute|power|energy|interconnect|networking|cuda|kernel|distributed training|colossus|stargate"),
    ("business", r"raises?|funding|series [a-h]\b|valuation|acquir|acquisition|merger|ipo\b|revenue|investment|investor|layoff|deal\b|partnership|pricing"),
    ("policy", r"regulat|policy|law\b|legislat|executive order|congress|senate|eu ai act|ai act|copyright|lawsuit|court|export control|ban\b|government|ministry|网信办|监管|政策|法规"),
    ("people", r"joins|hired|hires|departs|leaves|steps down|appointed|ceo\b|chief scientist|co-?founder|离职|加入|任命"),
    ("opensource", r"open[- ]source|open[- ]weight|apache 2\.0|mit license|weights released|开源"),
    ("product", r"launch|rolls out|now available|release[sd]?\b|introduc|app\b|feature|pricing|plan\b|发布|上线"),
]
TOPIC_RES = [(t, re.compile(p, re.I)) for t, p in TOPICS]
# 偏好加权（只用于排序，不是边界）。默认值是中性示例；个人偏好放在 fetch/preferences.json
# （或 DIGEST_PREFERENCES_FILE 指向的文件），格式见 preferences.example.json，该文件已被 .gitignore 忽略。
def _load_pref() -> dict:
    default = {"agents": 1, "training": 1, "reasoning": 1, "evals": 1, "inference": 1}
    p = os.environ.get("DIGEST_PREFERENCES_FILE") or str(HERE / "preferences.json")
    try:
        return {str(k): int(v) for k, v in json.loads(Path(p).read_text(encoding="utf-8")).items()}
    except (OSError, ValueError):
        return default


PREF = _load_pref()

TRACK_PARAMS = {"utm_source", "utm_medium", "utm_campaign", "utm_content", "utm_term", "ref", "ref_src",
                "source", "rss", "src", "fbclid", "gclid", "mc_cid", "mc_eid", "smid", "cmpid", "sr_share"}

lock = threading.Lock()
PRERELEASE_RE = re.compile(r"(?i)(alpha|beta|\brc\.?\d*\b|-rc|\.dev\d+|\bdev\d*\b|nightly|-pre\b|preview\.\d+|canary)")
# HF 衍生物：量化 / 合并 / 去审查 / 格式转换，不算新模型
DERIV_RE = re.compile(r"(?i)(gguf|\bawq\b|gptq|exl[23]|-mlx|mlx-|\d+-?bit\b|nvfp4|mxfp4|-fp[48]\b|-int[48]\b|\bq\d_k|\bq\dk\b|-q\d\b|abliterat|uncensor|heretic|\bmerge|frankenmerge|-lora\b|quantiz)")
# GitHub 刷星垃圾仓库：账号名「词+四位年份」、同描述批量克隆
SPAM_OWNER_RE = re.compile(r"^[a-z]+\d{4,}$|^[a-z]+[0-9]{2,}[a-z]+\d{2,}$", re.I)


@dataclass
class Cand:
    title: str
    url: str
    source: str
    channel: str
    kind: str
    published: str | None = None      # ISO8601（DIGEST_TZ 时区）；None = 未知（html_links 差分新出现）
    summary: str = ""
    authors: list[str] = field(default_factory=list)
    extra: dict = field(default_factory=dict)
    id: str = ""
    topics: list[str] = field(default_factory=list)
    pref: int = 0
    also_in: list[str] = field(default_factory=list)
    in_ledger: str | bool = False   # "pushed" / "seen" / False


# ───────────────────────── 工具函数 ─────────────────────────
class Ctx:
    wait_arxiv_minutes = 0   # cost-opt 2026-11-17: 默认不在 fetch 内等 arXiv（改由 routine 在写论文版面前补查一次）

    def __init__(self, settings, start, end, state, use_state, catchup, gh_token=None):
        self.user_agent = settings.get("user_agent", "Mozilla/5.0")
        self._local = threading.local()
        self._sessions: list[requests.Session] = []
        self.timeout = settings.get("timeout", 20)
        self.summary_chars = settings.get("summary_chars", 700)
        self.start, self.end = start, end
        self.state, self.use_state, self.catchup = state, use_state, catchup
        raw_tok = gh_token if gh_token is not None else (os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or "")
        self.gh_token = raw_tok.strip() or None
        self._gh_rate_warned = False
        self.notes: list[str] = []

    @property
    def s(self) -> requests.Session:
        sess = getattr(self._local, "session", None)
        if sess is None:
            sess = requests.Session()
            sess.headers["User-Agent"] = self.user_agent
            adapter = requests.adapters.HTTPAdapter(pool_connections=16, pool_maxsize=16)
            sess.mount("http://", adapter)
            sess.mount("https://", adapter)
            self._local.session = sess
            with lock:
                self._sessions.append(sess)
        return sess

    def add_note(self, msg: str) -> None:
        with lock:
            self.notes.append(msg)

    def close(self) -> None:
        with lock:
            sessions, self._sessions = self._sessions, []
        for sess in sessions:
            try:
                sess.close()
            except Exception:
                pass

    def get(self, url, *, accept=None, retries=2, params=None, headers=None, timeout=None):
        h = {}
        if accept:
            h["Accept"] = accept
        if headers:
            h.update(headers)
        last = None
        for i in range(retries + 1):
            try:
                r = self.s.get(url, headers=h, params=params, timeout=timeout or self.timeout)
                if r.status_code == 429 or r.status_code >= 500:
                    last = f"HTTP {r.status_code}"
                    time.sleep(2 + 3 * i)
                    continue
                if r.status_code != 200:
                    raise RuntimeError(f"HTTP {r.status_code}")
                return r
            except (requests.Timeout, requests.ConnectionError) as e:
                last = type(e).__name__
                time.sleep(1 + 2 * i)
        raise RuntimeError(last or "failed")

    def in_window(self, dt: datetime | None) -> bool:
        return dt is not None and self.start <= dt < self.end   # 左闭右开 [start,end)


def clean_text(s: str | None, n: int | None = None) -> str:
    if not s:
        return ""
    if "<" in s:
        try:
            s = lhtml.fromstring(f"<div>{s}</div>").text_content()
        except Exception:
            s = re.sub(r"<[^>]+>", " ", s)
    s = htmlmod.unescape(s)
    s = re.sub(r"\s+", " ", s).strip()
    if n and len(s) > n:
        s = s[: n - 1].rstrip() + "…"
    return s


def parse_dt(s: str | None) -> datetime | None:
    if not s:
        return None
    s = s.strip()
    try:
        d = parsedate_to_datetime(s)
        if d.tzinfo is None:
            d = d.replace(tzinfo=UTC)
        return d
    except Exception:
        pass
    t = s.replace("Z", "+00:00")
    t = re.sub(r"\s+([+-]\d{2}):?(\d{2})$", r"\1:\2", t)  # "2026-11-15 13:19:51  +0800"
    t = re.sub(r"([+-]\d{2})(\d{2})$", r"\1:\2", t)
    for cand in (t, t.replace(" ", "T", 1)):
        try:
            d = datetime.fromisoformat(cand)
            if d.tzinfo is None:
                d = d.replace(tzinfo=UTC)
            return d
        except Exception:
            continue
    m = re.match(r"(\d{4}-\d{2}-\d{2})", s)
    if m:
        return datetime.fromisoformat(m.group(1)).replace(tzinfo=UTC)
    return None


def iso(d: datetime | None) -> str | None:
    return d.astimezone(TZ_LOCAL).isoformat(timespec="minutes") if d else None


def canon_url(u: str) -> str:
    u = (u or "").strip()
    if not u:
        return u
    m = re.search(r"arxiv\.org/(?:abs|pdf|html)/(\d{4}\.\d{4,5})", u)
    if m:
        return f"arxiv:{m.group(1)}"
    m = re.search(r"huggingface\.co/papers/(\d{4}\.\d{4,5})", u)
    if m:
        return f"arxiv:{m.group(1)}"
    p = urlparse(u)
    q = [(k, v) for k, v in parse_qsl(p.query) if k.lower() not in TRACK_PARAMS and not k.lower().startswith("utm_")]
    host = p.netloc.lower().removeprefix("www.").removeprefix("m.")
    path = re.sub(r"/+$", "", p.path) or "/"
    return urlunparse(("https", host, path, "", urlencode(q), ""))


def norm_title(t: str) -> str:
    t = t.lower()
    t = re.sub(r"[^\w\u4e00-\u9fff]+", "", t)
    return t[:80]


def tag(c: Cand):
    text = f"{c.title} {c.summary[:400]}"
    c.topics = [t for t, r in TOPIC_RES if r.search(text)]
    if c.kind == "paper":   # 论文摘要里 release/deal/power 之类词太多，商业/产品/人事标签只给非论文
        c.topics = [t for t in c.topics if t not in ("product", "business", "people", "policy" if "policy" not in c.title.lower() else "")]
    c.pref = sum(PREF.get(t, 0) for t in c.topics)


def ai_ok(c: Cand) -> bool:
    return bool(AI_RE.search(f"{c.title} {c.summary[:300]}"))


MONTHS = {m: i for i, m in enumerate(["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"], 1)}


def date_from_text(t: str) -> datetime | None:
    """从链接文字 / URL 里猜日期（Oct 05, 2026 / 2026.10.22 / 2026-11/13 / news261113 / 13 Nov 2026 / 2026年11月13日）。"""
    if not t:
        return None
    m = re.search(r"(20\d\d)[./-](\d{1,2})[./-]?/?(\d{1,2})(?!\d)", t)
    if m:
        y, mo, d = map(int, m.groups())
    else:
        m = re.search(r"(20\d\d)年(\d{1,2})月(\d{1,2})日", t)
        if m:
            y, mo, d = map(int, m.groups())
        else:
            m = re.search(r"(?i)(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.? (\d{1,2}),? (20\d\d)", t)
            if m:
                mo, d, y = MONTHS[m.group(1).lower()], int(m.group(2)), int(m.group(3))
            else:
                m = re.search(r"(?i)(?<!\d)(\d{1,2}) (jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]* (20\d\d)", t)
                if m:
                    d, mo, y = int(m.group(1)), MONTHS[m.group(2).lower()], int(m.group(3))
                else:
                    m = re.search(r"/news(\d{2})(\d{2})(\d{2})$", t)   # DeepSeek news261113
                    if not m:
                        return None
                    y, mo, d = 2000 + int(m.group(1)), int(m.group(2)), int(m.group(3))
    try:
        # 只有日期：按当地中午近似，避免时区把它推到前一天
        return datetime(y, mo, d, 12, 0, tzinfo=UTC)
    except ValueError:
        return None


# ───────────────────────── 各类抓取器 ─────────────────────────
def xml_root(content: bytes):
    return etree.fromstring(content, etree.XMLParser(recover=True, huge_tree=True, resolve_entities=False))


def lname(e):
    return etree.QName(e).localname if isinstance(e.tag, str) else ""


def fetch_rss(ctx: Ctx, src: dict) -> list[Cand]:
    r = ctx.get(src["url"], accept="application/rss+xml,application/atom+xml,application/xml;q=0.9,*/*;q=0.8",
                headers={"User-Agent": src["user_agent"]} if src.get("user_agent") else None)
    root = xml_root(r.content)
    if root is None:
        raise RuntimeError("not XML")
    out = []
    n_items = 0
    for it in root.iter():
        if lname(it) not in ("item", "entry"):
            continue
        n_items += 1
        title = link = summ = None
        dt = None
        authors = []
        for ch in it:
            ln = lname(ch)
            if ln == "title":
                title = "".join(ch.itertext())
            elif ln == "link":
                href = ch.get("href")
                rel = ch.get("rel", "alternate")
                if href and rel == "alternate":
                    link = link or href
                elif ch.text and ch.text.strip():
                    link = link or ch.text.strip()
            elif ln in ("description", "summary", "encoded", "content") and not summ:
                summ = "".join(ch.itertext())
            elif ln == "group":  # youtube media:group
                for g in ch:
                    if lname(g) == "description" and not summ:
                        summ = g.text or ""
            elif ln in ("pubDate", "published", "date", "issued") and dt is None:
                dt = parse_dt(ch.text)
            elif ln == "updated" and dt is None:
                dt = parse_dt(ch.text)
            elif ln in ("author", "creator"):
                nm = "".join(ch.itertext()).strip()
                if nm:
                    authors.append(clean_text(nm, 80))
            elif ln == "guid" and not link and ch.text and ch.text.startswith("http"):
                link = ch.text.strip()
        if not title or not link:
            continue
        if not ctx.in_window(dt):
            continue
        c = Cand(clean_text(title, 300), urljoin(src["url"], link.strip()), src["id"], src["channel"], src["kind"],
                 iso(dt), clean_text(summ, ctx.summary_chars), authors[:6])
        if src.get("filter") == "ai" and not ai_ok(c):
            continue
        out.append(c)
    if n_items == 0:
        raise RuntimeError("feed has 0 items (dead or format changed)")
    return out


def fetch_html_links(ctx: Ctx, src: dict) -> list[Cand]:
    """无 RSS 的页面：取符合 pattern 的链接，和上次运行的已见集合做差分。首轮只建基线，返回空（除非 --no-state）。"""
    r = ctx.get(src["url"], accept="text/html", headers={"User-Agent": src["user_agent"]} if src.get("user_agent") else None)
    doc = lhtml.fromstring(r.content)
    pat = re.compile(src["pattern"])
    found: dict[str, str] = {}
    dates_hint: dict[str, datetime] = {}
    for a in doc.iter("a"):
        href = (a.get("href") or "").strip()
        if not href or not pat.search(href):
            continue
        full = urljoin(src["url"], href)
        if canon_url(full) == canon_url(src["url"]):
            continue
        if src.get("exclude") and re.search(src["exclude"], href):
            continue
        txt = clean_text(a.text_content(), 200)
        par = a.getparent()
        ctxt = clean_text(par.text_content(), 300) if par is not None else ""
        dguess = date_from_text(txt) or date_from_text(full) or date_from_text(ctxt)
        if full not in found or (len(txt) > len(found[full])):
            found[full] = txt
            if dguess:
                dates_hint[full] = dguess
    if not found:
        raise RuntimeError("0 links matched pattern (page changed?)")
    # state: {url: 首次看到的 epoch 秒}；首轮基线记为 0。窗口内首次出现的链接 = 新条目（重跑同一窗口结果一致）
    key = f"html:{src['id']}"
    seen = ctx.state.get(key, {})
    if isinstance(seen, list):
        seen = {u: 0 for u in seen}
    first = not seen
    now_ts = int(min(datetime.now(UTC), ctx.end).timestamp())
    lo = ctx.start.timestamp()
    new = [u for u in found if (u not in seen and not first) or (seen.get(u, 0) > lo)]
    with lock:
        merged = dict(seen)
        for u in found:
            if u not in merged:
                merged[u] = 0 if first else now_ts
        if len(merged) > 3000:
            merged = dict(sorted(merged.items(), key=lambda kv: kv[1])[-3000:])
        ctx.state[key] = merged
    if first or not ctx.use_state:
        # 首轮 / 无状态：基线化；把页面最前面的若干条以「日期未知」返回，供人工判断是否在窗口内
        new = list(found)[: src.get("first_run_keep", 6)]
    out = []
    hdr = {"User-Agent": src["user_agent"]} if src.get("user_agent") else None
    def resolve(u):
        """打开文章页取 og:title / description / 发布时间；失败就保持「日期未知」。"""
        try:
            d = lhtml.fromstring(ctx.get(u, accept="text/html", retries=0, headers=hdr, timeout=12).content)
        except Exception:
            return None, None, None
        def meta(*names):
            for n in names:
                v = d.xpath(f"//meta[@property='{n}' or @name='{n}' or @itemprop='{n}']/@content")
                if v and v[0].strip():
                    return v[0].strip()
            return None
        dt = parse_dt(meta("article:published_time", "og:published_time", "datePublished", "publish_date", "date", "pubdate", "parsely-pub-date"))
        if dt is None:
            t = d.xpath("//time/@datetime")
            dt = parse_dt(t[0]) if t else None
        if dt is None:
            for sc in d.xpath("//script[@type='application/ld+json']/text()"):
                m = re.search(r'"datePublished"\s*:\s*"([^"]+)"', sc)
                if m:
                    dt = parse_dt(m.group(1))
                    break
        return meta("og:title", "twitter:title"), meta("og:description", "description", "twitter:description"), dt
    # 已能从链接文字/URL 猜出日期且明显早于窗口的，直接丢掉，不再打开页面
    def stale(u):
        d = dates_hint.get(u)
        return d is not None and d < ctx.start - timedelta(days=1)
    new = [u for u in new if not stale(u)]
    todo = new[: src.get("resolve_max", 15)]
    with cf.ThreadPoolExecutor(6) as ex:
        resolved = list(ex.map(resolve, todo)) if src.get("resolve", True) else [(None, None, None)] * len(todo)
    for u, (t2, desc, dt) in zip(todo, resolved):
        title = clean_text(t2, 300) or found[u] or u.rstrip("/").rsplit("/", 1)[-1].replace("-", " ")
        if dt is None and u in dates_hint:
            dh = dates_hint[u]
            # 只有日期精度：窗口两端各放宽 12 小时
            if not (ctx.start - timedelta(hours=12) < dh <= ctx.end + timedelta(hours=12)):
                continue
            dt = dh
        if dt is not None and dt.tzinfo is None:
            dt = dt.replace(tzinfo=UTC)
        if dt is not None and not ctx.in_window(dt):
            # 有明确日期且不在窗口：首轮/无状态时丢弃；增量模式下若刚出现但日期早于窗口（补发/改版），保留并注明
            if first or not ctx.use_state or dt > ctx.end:
                continue
        if dt is None and first and ctx.use_state:
            continue   # 首轮建基线：日期未知的旧链接不输出
        c = Cand(title, u, src["id"], src["channel"], src["kind"], iso(dt), clean_text(desc, ctx.summary_chars),
                 extra={"date_unknown": dt is None, "baseline_run": first, "date_from": "page" if dt else None})
        if src.get("filter") == "ai" and not ai_ok(c):
            continue
        out.append(c)
    return out


def arxiv_announce_dt(listing_date: datetime) -> datetime:
    """arXiv 列表日期 D 在前一天 20:00 美东公开（周日至周四晚公告，对应周一至周五的列表）。
    2026-11-15 实测：「Friday, 13 November」列表里最晚的投稿是 11/12 13:59 EST，即周四 20:00 ET 公告
    = 本地周五 09:00（冬令时）/ 08:00（夏令时）。周六、周日本地没有新批次。"""
    d = listing_date - timedelta(days=1)
    return datetime(d.year, d.month, d.day, 20, 0, tzinfo=ET)


def _parse_arxiv_dl(doc, want_sections=("new", "cross")):
    """解析 /list/x/new 或 /pastweek 的 <dl>；返回 [(section_label, id, title, authors, subjects, abstract)]"""
    res = []
    for dl in doc.xpath("//dl[@id='articles']") or doc.xpath("//dl"):
        label = None
        for el in dl:
            if el.tag == "h3":
                t = el.text_content()
                label = t
                continue
            if el.tag == "dt":
                cur_id = None
                for a in el.iter("a"):
                    m = re.search(r"/abs/(\d{4}\.\d{4,5})", a.get("href") or "")
                    if m:
                        cur_id = m.group(1)
                        break
                res.append([label, cur_id, None, None, None, None])
            elif el.tag == "dd" and res:
                row = res[-1]
                t = el.xpath(".//div[contains(@class,'list-title')]")
                row[2] = clean_text(t[0].text_content().replace("Title:", "", 1)) if t else ""
                au = el.xpath(".//div[contains(@class,'list-authors')]//a")
                row[3] = [clean_text(a.text_content()) for a in au][:12]
                sj = el.xpath(".//div[contains(@class,'list-subjects')]")
                row[4] = clean_text(sj[0].text_content().replace("Subjects:", "", 1)) if sj else ""
                ab = el.xpath(".//p[contains(@class,'mathjax')]")
                row[5] = clean_text(ab[0].text_content()) if ab else ""
    return res


def fetch_arxiv(ctx: Ctx, src: dict) -> list[Cand]:
    cats = src["categories"]
    # 1) 用 cs.LG pastweek 拿到最近 5 个公告日（自动处理节假日）
    pw = {}
    def get_pw(cat):
        r = ctx.get(f"https://arxiv.org/list/{cat}/pastweek", params={"show": 2000}, timeout=60)
        return cat, lhtml.fromstring(r.content)
    with cf.ThreadPoolExecutor(4) as ex:
        for cat, doc in ex.map(get_pw, cats):
            pw[cat] = doc
    def day_of(label):
        m = re.search(r"(\w{3}), (\d{1,2}) (\w{3}) (\d{4})", label or "")
        return datetime.strptime(" ".join(m.groups()[1:]), "%d %b %Y") if m else None
    days = sorted({day_of(h.text_content()) for h in pw[cats[0]].xpath("//h3")} - {None})
    # 2026-11-16 教训：/pastweek 可能比 /new 晚刷新，所以同时看 /list/<cat>/new 的标题日期。
    # 2026-11-17 自然日窗口：期望批次 = 公告时刻落在 [start,end) 内的工作日列表（日报通常 1 批，周报 5 批）。
    # 周一至周五 09:00（夏令时 08:00）本地公告，必在 P 日窗口内；凌晨跑时一定已上线，不再等待/补收。
    # 工作日 P 没有批次 = 真实召回故障：note 里给 ⚠️，并在 20 秒后再查一次。
    now = datetime.now(UTC)
    cand_days = {datetime(x.year, x.month, x.day) for x in
                 (ctx.start.date() + timedelta(days=i) for i in range(-1, (ctx.end - ctx.start).days + 3))}
    exp_labels = sorted(L for L in cand_days if L.weekday() < 5 and ctx.start <= arxiv_announce_dt(L) < ctx.end)
    expected = bool(exp_labels)
    exp_label = exp_labels[-1] if expected else datetime(ctx.start.year, ctx.start.month, ctx.start.day)
    new_docs: dict = {}
    def new_date(doc):
        for h in doc.xpath("//h3"):
            m = re.search(r"new listings for \w+, (\d{1,2}) (\w+) (\d{4})", h.text_content())
            if m:
                return datetime.strptime(" ".join(m.groups()), "%d %B %Y")
        return None
    def get_new_doc(cat):
        r = ctx.get(f"https://arxiv.org/list/{cat}/new", params={"skip": 0, "show": 2000}, timeout=60)
        return cat, lhtml.fromstring(r.content)
    def have_exp():
        """最新的期望批次是否已在 /pastweek 或 /new 上线（/pastweek 可能比 /new 晚刷新，所以两处都看）。"""
        if days and days[-1] >= exp_label:
            return "pastweek"
        try:
            _, d0 = get_new_doc(cats[0])
            if new_date(d0) == exp_label:
                new_docs[cats[0]] = d0
                return "new"
        except Exception as e:
            ctx.add_note(f"arxiv /new check: {e}")
        return None
    via = None
    if expected:
        via = have_exp()
        if not via and arxiv_announce_dt(exp_label) <= now < ctx.end + timedelta(days=1):
            # 实时运行（凌晨）才重试一次；历史窗口（回填/测试）不重试
            time.sleep(20)
            try:
                _, doc = get_pw(cats[0])
                pw[cats[0]] = doc
                days = sorted({day_of(h.text_content()) for h in doc.xpath("//h3")} - {None})
            except Exception as e:
                ctx.add_note(f"arxiv pastweek retry: {e}")
            via = have_exp()
        if via == "pastweek":
            with cf.ThreadPoolExecutor(4) as ex:   # 其余分类重新拉取（可能刚刷新）
                for cat, doc in ex.map(get_pw, cats[1:]):
                    pw[cat] = doc
        if not via:
            age = now - arxiv_announce_dt(exp_label)
            if age > timedelta(days=5):
                ctx.add_note(tr("arxiv_rolled", label=f"{exp_label:%a %d %b}", days=age.days))
            else:
                ctx.add_note(tr("arxiv_missing", label=f"{exp_label:%a %d %b}", when=iso(arxiv_announce_dt(exp_label))))
        missing_prev = [L for L in exp_labels[:-1] if L not in days and (now - arxiv_announce_dt(L)) <= timedelta(days=5)]
        if missing_prev:
            ctx.add_note(tr("arxiv_missing_prev") + ", ".join(f"{L:%a %d %b}" for L in missing_prev))
    if via == "new":
        # /pastweek 还没刷新：直接用各分类 /new 页面的 New + Cross 两段作为这一批
        with cf.ThreadPoolExecutor(4) as ex:
            for cat, doc in ex.map(get_new_doc, [c for c in cats if c not in new_docs]):
                new_docs[cat] = doc
        if exp_label not in days:
            days = sorted(set(days) | {exp_label})
        ctx.add_note(tr("arxiv_from_new", label=f"{exp_label:%a %d %b}"))
    ingested = set(ctx.state.get("arxiv:ingested", []))
    targets = []
    for d in days:
        ann = arxiv_announce_dt(d)
        key = d.strftime("%Y-%m-%d")
        if ctx.start <= ann < ctx.end:
            targets.append((d, "window"))
        elif ctx.catchup and ctx.use_state and ann < ctx.start and ann > ctx.start - timedelta(days=4) and key not in ingested and ingested:
            targets.append((d, "catchup"))   # 仅 --catchup 时：补收更早漏掉的公告批次（自然日窗口下默认不需要）
    if not targets:
        latest = days[-1] if days else None
        raise_info = f"no announcement in window; latest listing {latest:%a %d %b} announced {iso(arxiv_announce_dt(latest))}" if latest else "no listing"
        ctx.add_note(f"arxiv: {raise_info}")
        return []
    target_keys = {d.strftime("%Y-%m-%d"): why for d, why in targets}
    # 2) 收集目标日的 id（每个分类的 pastweek 都按日期分段）
    meta: dict[str, dict] = {}
    exp_key = exp_label.strftime("%Y-%m-%d")
    if new_docs and exp_key in target_keys:
        for cat, doc in new_docs.items():
            if new_date(doc) != exp_label:
                continue
            for label, aid, title, authors, subj, ab in _parse_arxiv_dl(doc):
                if not aid or not label or "replacement" in label.lower():
                    continue
                m = meta.setdefault(aid, {"title": title, "authors": authors, "subjects": subj, "cats": [], "listing": exp_key, "why": target_keys[exp_key], "abstract": ab})
                if cat not in m["cats"]:
                    m["cats"].append(cat)
    for cat, doc in pw.items():
        rows = _parse_arxiv_dl(doc)
        for label, aid, title, authors, subj, _ in rows:
            d = day_of(label)
            if not aid or not d or d.strftime("%Y-%m-%d") not in target_keys:
                continue
            m = meta.setdefault(aid, {"title": title, "authors": authors, "subjects": subj, "cats": [], "listing": d.strftime("%Y-%m-%d"), "why": target_keys[d.strftime('%Y-%m-%d')]})
            if cat not in m["cats"]:
                m["cats"].append(cat)
    # 3) 摘要：最新一批直接用 /new 页面（含摘要），其余走 export API
    newest_day = max(target_keys)
    if newest_day == days[-1].strftime("%Y-%m-%d"):
        def get_new(cat):
            r = ctx.get(f"https://arxiv.org/list/{cat}/new", params={"skip": 0, "show": 2000}, timeout=60)
            return _parse_arxiv_dl(lhtml.fromstring(r.content))
        with cf.ThreadPoolExecutor(4) as ex:
            for rows in ex.map(get_new, cats):
                for label, aid, title, authors, subj, ab in rows:
                    if aid in meta and ab and not meta[aid].get("abstract"):
                        meta[aid]["abstract"] = ab
    missing = [a for a, m in meta.items() if not m.get("abstract")]
    for i in range(0, len(missing), 150):
        chunk = missing[i:i + 150]
        try:
            r = ctx.get("https://export.arxiv.org/api/query", params={"id_list": ",".join(chunk), "max_results": len(chunk)}, timeout=90)
            root = xml_root(r.content)
            for e in root.iter():
                if lname(e) != "entry":
                    continue
                eid = summ = None
                for ch in e:
                    if lname(ch) == "id":
                        mm = re.search(r"(\d{4}\.\d{4,5})", ch.text or "")
                        eid = mm.group(1) if mm else None
                    elif lname(ch) == "summary":
                        summ = clean_text(ch.text)
                if eid in meta and summ:
                    meta[eid]["abstract"] = summ
        except Exception as e:
            ctx.add_note(f"arxiv export API chunk {i}: {e}")
        time.sleep(3)
    out = []
    for aid, m in meta.items():
        ann = arxiv_announce_dt(datetime.strptime(m["listing"], "%Y-%m-%d"))
        out.append(Cand(m["title"] or aid, f"https://arxiv.org/abs/{aid}", "arxiv", "paper", "paper", iso(ann),
                        clean_text(m.get("abstract"), ctx.summary_chars), m["authors"] or [],
                        extra={"arxiv_id": aid, "cats": m["cats"], "subjects": m["subjects"], "listing": m["listing"], "batch": m["why"]}))
    with lock:
        ctx.state["arxiv:ingested"] = sorted(ingested | set(target_keys))[-60:]
    ctx.add_note("arxiv batches: " + ", ".join(f"{k}({v}, {tr('arxiv_pub', when=iso(arxiv_announce_dt(datetime.strptime(k, '%Y-%m-%d'))))})" for k, v in sorted(target_keys.items())))
    return out


def fetch_hf_daily(ctx: Ctx, src: dict) -> list[Cand]:
    """HF daily_papers?date=D 是「UTC 日期 D 策展的榜单」，整榜 submittedOnDailyAt 都盖成 D T00:00Z，
    但榜单在 UTC 日 D 内（本地 D 08:00 → D+1 08:00）逐步填满，D+1 00:00Z（本地 D+1 08:00）收榜。
    所以不用 submittedOnDailyAt 做时间窗过滤，而是按「已完整收榜」的 UTC 日期取整榜（2026-11-17 起的规则）：
    - 日报（读写状态、非 --start/--end 自定义窗口、非周报）：取「截至运行时刻已收榜、且 state['hf_daily:last'] 之后还没取过」
      的全部 UTC 日期，归入本期；没有 last 记录时回退为「收榜时刻落在窗口内」的日期。
      例：P 日凌晨 00:30（本地）运行 → 收榜到 UTC P-1 为止（P-1 在 P 08:00 收榜），所以 P 期取 UTC P-2 之后的新日期；周末/漏跑的空档自动补上，最多回溯 14 天。
    - 周报 / 自定义窗口 / --no-state：收榜时刻落在窗口 [start,end) 内的 UTC 日期（周报 = 7 个 UTC 日），不读写 state。
    - 当天（UTC）还没收榜的日期一律不取；HTTP 400（日期尚未开放）直接停，不让整个信源失败。
    - 成功取到的最后一个连续日期记入 state['hf_daily:last']。
    - 与 arXiv 的去重在 dedupe() 里按 arXiv id 合并（也标 also_in），与 ledger 的去重在 in_ledger。
    条目的 published 记为收榜时刻（D+1 00:00Z，本地 D+1 08:00）。"""
    end_u = ctx.end.astimezone(UTC)
    start_u = ctx.start.astimezone(UTC)
    closes = lambda d: datetime(d.year, d.month, d.day, tzinfo=UTC) + timedelta(days=1)
    # --only 补查（如 arXiv 缺批次时 --only arxiv,hf_daily）不读写 hf_daily:last：按窗口取同一份 UTC P-1 整榜，
    # 否则主抓取已推进 last，补查目录里会没有 HF 论文（2026-11-18 review）
    gap_ok = ctx.use_state and not getattr(ctx, "weekly", False) and not getattr(ctx, "custom_window", False) and not getattr(ctx, "only_mode", False)
    now_u = datetime.now(UTC)
    # 日报实时运行：cutoff = 运行时刻（只取已收榜的）；回填/历史窗口：cutoff = 窗口结束
    cutoff = now_u if (gap_ok and timedelta(0) <= now_u - end_u < timedelta(hours=12)) else end_u
    dates = set()
    d = start_u.date() - timedelta(days=1)
    while closes(d) <= cutoff:
        if start_u <= closes(d) < end_u:
            dates.add(d)
        d += timedelta(days=1)
    last = None
    if gap_ok and ctx.state.get("hf_daily:last"):
        last = datetime.strptime(ctx.state["hf_daily:last"], "%Y-%m-%d").date()
        d = last + timedelta(days=1)
        while closes(d) <= cutoff and d >= cutoff.date() - timedelta(days=14):
            dates.add(d)
            d += timedelta(days=1)
        dates = {x for x in dates if x > last}   # 已取过的不再取
    out, fetched, counts = [], [], {}
    for d in sorted(dates):
        try:
            r = ctx.get(src["url"], params={"date": d.isoformat(), "limit": 100})
        except RuntimeError as e:
            if "HTTP 400" in str(e):   # HF 拒绝「还没到的日期」（UTC），跳过该日即可，不让整个信源失败
                ctx.add_note(tr("hf_not_open", d=d))
                break
            raise
        items = r.json()
        fetched.append(d)
        counts[d.isoformat()] = len(items)
        pub = iso(closes(d).astimezone(TZ_LOCAL))
        for it in items:
            p = it.get("paper", {})
            aid = p.get("id")
            if not aid:
                continue
            c = Cand(clean_text(p.get("title") or it.get("title")), f"https://huggingface.co/papers/{aid}", src["id"], "paper", "paper",
                     pub, clean_text(p.get("ai_summary") or p.get("summary"), ctx.summary_chars),
                     [a.get("name", "") for a in p.get("authors", [])][:12],
                     extra={"arxiv_id": aid, "github": p.get("githubRepo"), "project": p.get("projectPage"),
                            "org": (p.get("organization") or {}).get("fullname"), "hf_date": d.isoformat()})
            out.append(c)
    ctx.add_note((tr("hf_utc") + ", ".join(f"{k}({v})" for k, v in counts.items())) if counts else tr("hf_none"))
    if gap_ok and fetched:
        new_last = max(fetched + ([last] if last else []))
        with lock:
            ctx.state["hf_daily:last"] = new_last.isoformat()
    return out


def fetch_hf_org_models(ctx: Ctx, src: dict) -> list[Cand]:
    orgs = list(dict.fromkeys(src["orgs"]))
    errs = []
    def one(org):
        try:
            r = ctx.get("https://huggingface.co/api/models", params={"author": org, "sort": "createdAt", "direction": -1, "limit": 25, "full": "false"}, retries=1)
            return org, r.json()
        except Exception as e:
            errs.append(f"{org}: {e}")
            return org, []
    out = []
    with cf.ThreadPoolExecutor(12) as ex:
        for org, items in ex.map(one, orgs):
            for m in items:
                dt = parse_dt(m.get("createdAt"))
                if not ctx.in_window(dt):
                    continue
                mid = m.get("id") or m.get("modelId")
                if DERIV_RE.search(mid or ""):
                    continue
                tags = [t for t in m.get("tags", []) if not t.startswith(("region:", "endpoints", "autotrain"))][:12]
                out.append(Cand(mid, f"https://huggingface.co/{mid}", src["id"], "model", "model", iso(dt),
                                f"pipeline={m.get('pipeline_tag')}; library={m.get('library_name')}; tags={', '.join(tags)}",
                                extra={"org": org, "pipeline": m.get("pipeline_tag")}))
    if errs:
        ctx.add_note(f"hf_org_models errors ({len(errs)}): " + "; ".join(errs[:8]))
    return out


def fetch_hf_trending(ctx: Ctx, src: dict) -> list[Cand]:
    r = ctx.get(src["url"])
    cutoff = ctx.end - timedelta(days=src.get("max_age_days", 21))
    base = "https://huggingface.co/" + ("datasets/" if "datasets" in src["url"] else "spaces/" if "spaces" in src["url"] else "")
    out = []
    for m in r.json():
        dt = parse_dt(m.get("createdAt"))
        if not dt or dt < cutoff or dt > ctx.end:
            continue
        mid = m.get("id")
        if DERIV_RE.search(mid or ""):
            continue
        desc = m.get("cardData", {}).get("short_description") if isinstance(m.get("cardData"), dict) else ""
        tags = [t for t in m.get("tags", []) if not t.startswith(("region:", "endpoints"))][:10]
        out.append(Cand(mid, base + mid, src["id"], "model", src["kind"], iso(dt),
                        clean_text(desc or f"pipeline={m.get('pipeline_tag')}; tags={', '.join(tags)}", 300),
                        extra={"created_in_window": ctx.in_window(dt), "discovery_only": True}))
    return out


def fetch_github_trending(ctx: Ctx, src: dict) -> list[Cand]:
    out = []
    for url in src["urls"]:
        try:
            r = ctx.get(url, accept="text/html")
        except Exception as e:
            ctx.add_note(f"gh_trending {url}: {e}")
            continue
        doc = lhtml.fromstring(r.content)
        for art in doc.xpath("//article[contains(@class,'Box-row')]"):
            a = art.xpath(".//h2//a")
            if not a:
                continue
            repo = re.sub(r"\s+", "", a[0].get("href", "")).strip("/")
            p = art.xpath(".//p")
            desc = clean_text(p[0].text_content()) if p else ""
            lang = art.xpath(".//*[@itemprop='programmingLanguage']")
            c = Cand(repo, f"https://github.com/{repo}", src["id"], "repo", "repo", iso(ctx.end - timedelta(minutes=1)), desc,
                     extra={"language": clean_text(lang[0].text_content()) if lang else None, "trending_list": url.split("github.com/")[1], "discovery_only": True})
            if src.get("filter") == "ai" and not ai_ok(c):
                continue
            out.append(c)
    return out


def fetch_github_releases(ctx: Ctx, src: dict) -> list[Cand]:
    errs = []
    def one(repo):
        try:
            r = ctx.get(f"https://github.com/{repo}/releases.atom", retries=1)
            root = xml_root(r.content)
        except Exception as e:
            errs.append(f"{repo}: {e}")
            return []
        res = []
        for e in root.iter():
            if lname(e) != "entry":
                continue
            title = link = summ = None
            dt = None
            for ch in e:
                ln = lname(ch)
                if ln == "title":
                    title = ch.text
                elif ln == "link":
                    link = ch.get("href")
                elif ln == "updated":
                    dt = parse_dt(ch.text)
                elif ln == "content":
                    summ = ch.text
            if ctx.in_window(dt) and title:
                if repo != "ggml-org/llama.cpp" and PRERELEASE_RE.search(title.strip() + " " + (link or "").rsplit("/", 1)[-1]):
                    continue
                res.append(Cand(f"{repo} {clean_text(title, 120)}", link or f"https://github.com/{repo}/releases", src["id"], "tooling", "release",
                                iso(dt), clean_text(summ, ctx.summary_chars), extra={"repo": repo}))
        if len(res) > 1:   # 同一仓库窗口内多次发布：只留最新 1 条，其余记数（llama.cpp 构建号、langgraph 子包等）
            n = len(res)
            res = res[:1]
            res[0].extra["releases_in_window"] = n
        return res
    out = []
    with cf.ThreadPoolExecutor(8) as ex:
        for items in ex.map(one, list(dict.fromkeys(src["repos"]))):
            out.extend(items)
    if errs:
        ctx.add_note(f"gh_releases errors ({len(errs)}): " + "; ".join(errs[:10]))
    return out


def _gh_api(ctx: Ctx, url: str, params=None):
    tok = (ctx.gh_token or os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or "").strip()
    h = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if tok:
        h["Authorization"] = f"Bearer {tok}"
    last = None
    for i in range(2):
        try:
            r = ctx.s.get(url, headers=h, params=params, timeout=ctx.timeout)
        except (requests.Timeout, requests.ConnectionError) as e:
            last = type(e).__name__
            time.sleep(2 + 3 * i)
            continue
        if r.status_code in (403, 429):
            rem = r.headers.get("X-RateLimit-Remaining", "")
            body_low = (r.text or "")[:300].lower()
            is_rl = (
                r.status_code == 429
                or rem == "0"
                or "rate limit" in body_low
                or "secondary rate" in body_low
            )
            warn = tr("gh_rate_auth" if tok else "gh_rate_unauth", code=r.status_code)
            with lock:
                if not getattr(ctx, "_gh_rate_warned", False):
                    ctx._gh_rate_warned = True
                    ctx.notes.append(warn)
            detail = "rate limit" if is_rl else "forbidden/rate limit"
            hint = "check GITHUB_TOKEN/GH_TOKEN" if tok else "set GITHUB_TOKEN or GH_TOKEN"
            raise RuntimeError(f"HTTP {r.status_code} ({detail}; {hint})")
        if r.status_code >= 500:
            last = f"HTTP {r.status_code}"
            time.sleep(2 + 3 * i)
            continue
        if r.status_code != 200:
            raise RuntimeError(f"HTTP {r.status_code}")
        return r
    raise RuntimeError(last or "GitHub API request failed")


def fetch_github_org_repos(ctx: Ctx, src: dict) -> list[Cand]:
    errs = []
    orgs = list(dict.fromkeys(src["orgs"]))
    def one(org):
        try:
            r = _gh_api(ctx, f"https://api.github.com/orgs/{org}/repos", {"sort": "created", "direction": "desc", "per_page": 8})
            return org, r.json()
        except Exception as e:
            errs.append(f"{org}: {e}")
            return org, []
    out = []
    with cf.ThreadPoolExecutor(6) as ex:
        for org, items in ex.map(one, orgs):
            for it in items if isinstance(items, list) else []:
                dt = parse_dt(it.get("created_at"))
                if not ctx.in_window(dt) or it.get("fork"):
                    continue
                out.append(Cand(it["full_name"], it["html_url"], src["id"], "repo", "repo", iso(dt), clean_text(it.get("description"), 400), extra={"org": org}))
    if errs:
        ctx.add_note(f"gh_org_repos errors ({len(errs)}): " + "; ".join(errs[:6]))
        if not out and orgs and len(errs) == len(orgs):
            raise RuntimeError(errs[0])
    return out


def fetch_github_search(ctx: Ctx, src: dict) -> list[Cand]:
    since3 = (ctx.end - timedelta(days=3)).astimezone(UTC).strftime("%Y-%m-%d")
    out = []
    errs = []
    seen_desc: set[str] = set()
    queries = src["queries"]
    for idx, q in enumerate(queries):
        q = q.replace("{since3}", since3)
        try:
            r = _gh_api(ctx, "https://api.github.com/search/repositories", {"q": q, "sort": "stars", "order": "desc", "per_page": 50})
        except Exception as e:
            errs.append(f"q{idx + 1}: {e}")
            continue
        for it in r.json().get("items", []):
            owner = it["full_name"].split("/")[0]
            dnorm = norm_title(it.get("description") or "")[:40]
            if SPAM_OWNER_RE.match(owner) or (dnorm and dnorm in seen_desc):
                continue
            if dnorm:
                seen_desc.add(dnorm)
            dt = parse_dt(it.get("created_at"))
            c = Cand(it["full_name"], it["html_url"], src["id"], "repo", "repo", iso(dt), clean_text(it.get("description"), 400),
                     extra={"language": it.get("language"), "topics": it.get("topics", [])[:8], "discovery_only": True})
            if not ai_ok(Cand(c.title + " " + " ".join(c.extra["topics"]), "", "", "", "", summary=c.summary)):
                continue
            out.append(c)
        if idx + 1 < len(queries):
            time.sleep(1)
    if errs:
        ctx.add_note(f"gh_search errors ({len(errs)}): " + "; ".join(errs[:4]))
        if not out and queries and len(errs) == len(queries):
            raise RuntimeError(errs[0])
    return out


def fetch_hn(ctx: Ctx, src: dict) -> list[Cand]:
    out = []
    page = 0
    lo, hi = int(ctx.start.timestamp()), int(ctx.end.timestamp())
    while page < 10:
        r = ctx.get("https://hn.algolia.com/api/v1/search_by_date",
                    params={"tags": "story", "numericFilters": f"created_at_i>{lo},created_at_i<={hi},points>={src.get('min_points', 25)}", "hitsPerPage": 500, "page": page})
        j = r.json()
        for h in j.get("hits", []):
            title = h.get("title") or ""
            url = h.get("url") or f"https://news.ycombinator.com/item?id={h['objectID']}"
            c = Cand(clean_text(title), url, src["id"], "community", "discussion", iso(datetime.fromtimestamp(h["created_at_i"], UTC)),
                     clean_text(h.get("story_text"), 300), extra={"hn": f"https://news.ycombinator.com/item?id={h['objectID']}"})
            if ai_ok(Cand(c.title + " " + urlparse(url).netloc, "", "", "", "")):
                out.append(c)
        page += 1
        if page >= j.get("nbPages", 0):
            break
    return out


def fetch_reddit(ctx: Ctx, src: dict) -> list[Cand]:
    # reddit 对并发很敏感：串行 + 退避
    with reddit_lock:
        time.sleep(2)
        r = ctx.get(src["url"], accept="application/atom+xml", retries=3)
    root = xml_root(r.content)
    out = []
    for e in root.iter():
        if lname(e) != "entry":
            continue
        title = link = summ = None
        dt = None
        for ch in e:
            ln = lname(ch)
            if ln == "title":
                title = ch.text
            elif ln == "link":
                link = ch.get("href")
            elif ln in ("updated", "published") and dt is None:
                dt = parse_dt(ch.text)
            elif ln == "content":
                summ = ch.text
        if ctx.in_window(dt) and title:
            s = clean_text(summ, ctx.summary_chars)
            s = re.sub(r"submitted by /u/\S+.*$", "", s).strip()  # 去掉用户名
            out.append(Cand(clean_text(title, 300), link, src["id"], "community", "discussion", iso(dt), s))
    return out


reddit_lock = threading.Lock()
FETCHERS = {
    "rss": fetch_rss, "html_links": fetch_html_links, "arxiv": fetch_arxiv, "hf_daily": fetch_hf_daily,
    "hf_org_models": fetch_hf_org_models, "hf_trending": fetch_hf_trending, "github_trending": fetch_github_trending,
    "github_releases": fetch_github_releases, "github_org_repos": fetch_github_org_repos,
    "github_search": fetch_github_search, "hn": fetch_hn, "reddit": fetch_reddit,
}


# ───────────────────────── 去重 / 账本 / 输出 ─────────────────────────
def _work_root() -> Path:
    env = os.environ.get("DIGEST_ROOT", "").strip()
    return Path(env) if env else ROOT


def _state_dir() -> Path:
    env = os.environ.get("DIGEST_ROOT", "").strip()
    return (Path(env) / "state") if env else (HERE / "state")


def _ledger_path() -> Path:
    env = os.environ.get("LEDGER_PATH", "").strip()
    return Path(env) if env else (_work_root() / "ledger.json")


def load_ledger_keys() -> dict[str, str]:
    """ledger.json → {key: 'pushed' | 'seen'}；category/status 含 seen 的是「看过未推」。"""
    keys: dict[str, str] = {}
    p = _ledger_path()
    try:
        d = json.loads(p.read_text(encoding="utf-8"))
        for it in d.get("items", []):
            st = "seen" if "seen" in f"{it.get('status') or ''} {it.get('category') or ''}" else "pushed"
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


KIND_RANK = {"lab_post": 0, "paper": 1, "model": 2, "release": 3, "repo": 4, "policy": 5, "essay": 6, "podcast": 7, "video": 8, "news": 9, "dataset": 10, "discussion": 11}


def dedupe(cands: list[Cand]) -> list[Cand]:
    by_key: dict[str, Cand] = {}
    by_title: dict[str, Cand] = {}
    out: list[Cand] = []
    for c in sorted(cands, key=lambda c: KIND_RANK.get(c.kind, 99)):
        k = canon_url(c.url)
        aid = c.extra.get("arxiv_id")
        if aid:
            k = f"arxiv:{aid}"
        t = norm_title(c.title)
        hit = by_key.get(k) or (by_title.get(t) if len(t) >= 16 else None)
        if hit:
            if c.source not in hit.also_in and c.source != hit.source:
                hit.also_in.append(c.source)
                if c.source == "hf_daily":
                    hit.extra["hf_daily"] = True
                    for kk in ("github", "project"):
                        if c.extra.get(kk):
                            hit.extra[kk] = c.extra[kk]
            if not hit.summary and c.summary:
                hit.summary = c.summary
            continue
        c.id = hashlib.sha1(k.encode()).hexdigest()[:12]
        by_key[k] = c
        if len(t) >= 16:
            by_title[t] = c
        out.append(c)
    return out


SEEN_PATH = HERE / "state" / "triage-seen.json"


def ckey(c) -> str:
    aid = c.extra.get("arxiv_id") if isinstance(c.extra, dict) else None
    return f"arxiv:{aid}" if aid else canon_url(c.url)


def _triage_seen_path() -> Path:
    if os.environ.get("DIGEST_ROOT", "").strip():
        return _state_dir() / "triage-seen.json"
    return SEEN_PATH


def load_triage_seen() -> dict[str, str]:
    """discovery-only 条目（trending / GitHub 搜索）首次出现在哪天的 triage：同一条以后只折叠成标题。"""
    try:
        return json.loads(_triage_seen_path().read_text(encoding="utf-8"))
    except Exception:
        return {}


def save_triage_seen(cands, label: str, seen: dict[str, str]):
    new = dict(seen)
    for c in cands:
        if c.extra.get("discovery_only"):
            new.setdefault(ckey(c), label)
    cut = (datetime.strptime(label, "%Y-%m-%d") - timedelta(days=45)).strftime("%Y-%m-%d")
    new = {k: v for k, v in new.items() if v >= cut}
    sp = _triage_seen_path()
    sp.parent.mkdir(parents=True, exist_ok=True)
    sp.write_text(json.dumps(new, ensure_ascii=False), encoding="utf-8")


SEC_KEYS = ["lab", "model", "tooling", "repo", "research_blog", "newsletter", "podcast",
            "chips_infra", "policy", "news", "news_zh", "community"]


def sec_order():
    return [(k, tr("sec_" + k)) for k in SEC_KEYS]

RICH_CH = {"lab", "tooling", "policy", "research_blog", "newsletter", "podcast"}
SNIP_CH = {"news", "news_zh", "community", "chips_infra"}   # 这些频道权重 ≥3 只加 100 字摘要，不带 URL


# ── 论文「其余」档折叠：无任何信号的方向外论文只留一行计数 + 全量标题文件；有信号的保留标题 ──
ARXIV_ID_RE = re.compile(r"(?<!\d)(\d{4}\.\d{4,5})(?!\d)")
# 大结果/前沿类关键词（只扫「其余」档的标题+摘要；刻意收紧，避免 interpretability/benchmark 这类泛词把折叠档又撑回去）
BIG_KW = [
    ("big_tech_report", r"technical report|tech report"),
    ("big_frontier", r"\b(?:gpt-?\d|chatgpt|claude|gemini|llama-?\d|qwen ?\d|deepseek|kimi|glm-?\d|mistral|grok)\b"),
    ("big_open", r"open[- ]weights?|open[- ]source (?:model|llm|foundation model)|model family|we (?:release|open-?source)\b.{0,50}(?:model|weights|checkpoint)"),
    ("big_scale", r"\btrillion\b|scaling laws?"),
    ("big_bench", r"swe-?bench|terminal-?bench|arc-?agi|humanity.s last exam|frontiermath|\bgpqa\b|\bputnam\b|\bimo\b|erd[oő]s|open problem|solves? (?:an )?open"),
    ("big_safety", r"jailbreak|backdoor|alignment[- ]faking|scheming|sabotag|reward hacking|red[- ]team|sleeper agent|prompt injection"),
]
BIG_KW_RE = [(n, re.compile(p, re.I)) for n, p in BIG_KW]   # n = i18n key (big_*)


def paper_signals(c, xref: dict[str, str]) -> list[str]:
    """返回「其余」档论文的保留理由；空列表 = 可折叠。"""
    tags = []
    if c.source == "hf_daily" or "hf_daily" in c.also_in:
        org = (c.extra or {}).get("org")
        tags.append("HF" + (f"·{org[:24]}" if org else ""))
    other = [x for x in c.also_in if x not in ("arxiv", "hf_daily")] + ([c.source] if c.source not in ("arxiv", "hf_daily") else [])
    if other:
        tags.append(tr("sig_multi") + ",".join(other[:3]))
    aid = (c.extra or {}).get("arxiv_id")
    if aid and aid in xref:
        tags.append(tr("sig_mentioned") + xref[aid][:30])
    if c.in_ledger:
        tags.append(tr("sig_ledger"))
    text = f"{c.title} {c.summary or ''}"
    hits = [n for n, rx in BIG_KW_RE if rx.search(text)]
    if hits:
        tags.append(tr("sig_kw") + "/".join(tr(h) for h in hits[:2]))
    return tags


def write_triage_v2(outdir: Path, cands: list[Cand], ctx: Ctx):
    """紧凑版 triage：每条一行；重要频道带 URL+短摘要，其余只列标题+短 id（`python3 fetch/cand.py <id>` 取 URL/摘要）。
    已推过（★）与「近日已列过的 trending」折叠成标题段落——折叠不是丢弃，标题仍在清单里，须扫一遍。"""
    day = getattr(ctx, "label_day", "")
    seen = getattr(ctx, "seen_prior", {})
    head_a = tr("head_a")
    head = head_a + (";" if FL == "en" else "；") + " " * (FL == "en") + tr("head_b")
    def when(c):
        return (c.published or "?")[5:16].replace("T", " ")
    def sid(c):
        return c.id[:6]
    def rich(c):
        sm = f"\n  - {c.summary[:160]}" if c.summary else ""
        star = "☆ " if c.in_ledger == "seen" else ""
        extra = f" (+{','.join(c.also_in)})" if c.also_in else ""
        return f"- {star}[{c.pref}] {c.title} — `{c.source}{extra} · {when(c)}` {c.url} #{sid(c)}{sm}"
    def mid(c):
        star = "☆ " if c.in_ledger == "seen" else ""
        extra = f"(+{','.join(c.also_in)})" if c.also_in else ""
        tops = ",".join(c.topics[:2])
        w = f"[{c.pref}] " if c.pref else ""
        sn = f"\n  - {c.summary[:100]}" if (c.pref >= 3 and c.summary and c.channel in SNIP_CH) else ""
        return f"- {star}{w}{c.title} — {c.source}{extra} {when(c)} {tops} #{sid(c)}{sn}"
    T = [tr("title_nonpaper", start=iso(ctx.start), end=iso(ctx.end)), "", head, ""]
    n_all = 0
    for ch, label in sec_order():
        items = [c for c in cands if c.channel == ch]
        if not items:
            continue
        items.sort(key=lambda c: (-c.pref, c.published or ""))
        pushed = [c for c in items if c.in_ledger == "pushed"]
        rep = [c for c in items if c.in_ledger != "pushed" and c.extra.get("discovery_only") and seen.get(ckey(c), day) < day]
        skip = {id(c) for c in pushed} | {id(c) for c in rep}
        rest = [c for c in items if id(c) not in skip]
        n_all += len(items)
        T.append(f"## {label}{i18n.cnt_paren(FL, len(items))}")
        T.append("")
        for c in rest:
            use_rich = (ch in RICH_CH or c.kind in ("lab_post", "essay", "release") or (c.pref >= 3 and ch not in SNIP_CH and not c.extra.get("discovery_only")))
            T.append(rich(c) if use_rich else mid(c))
        if pushed:
            T.append("")
            T.append(tr("pushed_line") + " ｜ ".join(f"{c.title} #{sid(c)}" for c in pushed))
        if rep:
            T.append("")
            T.append(tr("rep_line", n=len(rep)) + " ｜ ".join(f"{c.title} #{sid(c)}" for c in rep))
        T.append("")
    (outdir / "triage.md").write_text("\n".join(T) + "\n", encoding="utf-8")
    papers = [c for c in cands if c.channel == "paper"]
    P = [tr("title_papers", start=iso(ctx.start), end=iso(ctx.end), n=len(papers)), "", head_a + tr("papers_intro_tail"),
         tr("papers_group_note"), ""]
    def pl(c, summ=0):
        aid = c.extra.get("arxiv_id") or c.url
        hf = " ·HF" if (c.source == "hf_daily" or "hf_daily" in c.also_in) else ""
        star = {"pushed": "★ ", "seen": "☆ "}.get(c.in_ledger or "", "")
        t = f"- {aid} {star}{c.title}{hf}"
        if summ and c.summary:
            t += f"\n  - {c.summary[:summ]}"
        return t
    used = set()
    for label, fn, summ in [(tr("group_pref3"), lambda c: c.pref >= 3, 180), (tr("group_pref12"), lambda c: 1 <= c.pref <= 2, 0)]:
        items = [c for c in papers if fn(c) and c.id not in used]
        if not items:
            continue
        items.sort(key=lambda c: (-c.pref, -int(bool(c.also_in)), c.title))
        P.append(f"## {label}{i18n.cnt_paren(FL, len(items))}")
        if summ:
            P.extend(pl(c, summ) + f" [{c.pref}]" for c in items)
        else:   # 权重 1–2：按权重分小节，行内不再重复 [n]
            for pv in sorted({c.pref for c in items}, reverse=True):
                P.append(f"### {tr('weight_n', n=pv)}")
                P.extend(pl(c) for c in items if c.pref == pv)
        P.append("")
        used.update(c.id for c in items)
    rest = [c for c in papers if c.id not in used]

    def by_topic(items):
        bt: dict[str, list[Cand]] = {}
        for c in items:
            bt.setdefault(c.topics[0] if c.topics else tr("other"), []).append(c)
        return bt

    if rest and getattr(ctx, "fold_rest", True):
        xref: dict[str, str] = {}
        for c in cands:
            if c.channel != "paper":
                for m in ARXIV_ID_RE.finditer(f"{c.url} {c.title} {c.summary or ''}"):
                    xref.setdefault(m.group(1), f"{c.source}")
        keep, fold = [], []
        for c in rest:
            tg = paper_signals(c, xref)
            (keep if tg else fold).append((c, tg))
        # 全量标题（含被折叠的）写到旁边的文件，供 rg 查询
        R = [tr("rest_full", n=len(rest), k=len(keep), f=len(fold)), ""]
        for tname, its in sorted(by_topic([c for c, _ in keep + fold]).items(), key=lambda kv: -len(kv[1])):
            R.append(f"### {tname}{i18n.cnt_paren(FL, len(its))}")
            R.extend(pl(c) for c in sorted(its, key=lambda c: c.title))
        (outdir / "triage-papers-rest.md").write_text("\n".join(R) + "\n", encoding="utf-8")
        if keep:
            P.append("## " + tr("rest_keep", n=len(keep)))
            for c, tg in sorted(keep, key=lambda x: (x[1][0].split(":")[0] != "HF", x[0].title)):
                P.append(pl(c) + f" 〔{'; '.join(tg)}〕")
            P.append("")
        P.append("## " + tr("rest_fold", n=len(fold)))
        P.append(tr("fold_note", path=f"{outdir}/triage-papers-rest.md"))
        P.append("")
    else:
        bytopic = by_topic(rest)
        if rest:
            P.append("## " + tr("rest_plain", n=len(rest)))
            for tname in sorted(bytopic, key=lambda k: -len(bytopic[k])):
                P.append(f"### {tname}{i18n.cnt_paren(FL, len(bytopic[tname]))}")
                P.extend(pl(c) for c in sorted(bytopic[tname], key=lambda c: c.title))
            P.append("")
    (outdir / "triage-papers.md").write_text("\n".join(P) + "\n", encoding="utf-8")


def rebuild_triage(src_dir: Path, out: Path | None, a) -> int:
    rows = [json.loads(l) for l in (src_dir / "candidates.jsonl").read_text(encoding="utf-8").splitlines() if l.strip()]
    meta = json.loads((src_dir / "report.json").read_text(encoding="utf-8")) if (src_dir / "report.json").exists() else {}
    cands = [Cand(**{k: v for k, v in r.items()}) for r in rows]
    class _C: pass
    ctx = _C()
    ctx.start = datetime.fromisoformat(meta.get("window_start", "2000-01-01T00:00+00:00"))
    ctx.end = datetime.fromisoformat(meta.get("window_end", "2000-01-02T00:00+00:00"))
    ctx.label_day = (ctx.end - timedelta(microseconds=1)).strftime("%Y-%m-%d")
    ctx.seen_prior = load_triage_seen()
    ctx.fold_rest = not getattr(a, "no_fold", False)
    out = out or src_dir
    out.mkdir(parents=True, exist_ok=True)
    write_triage_v2(out, cands, ctx)
    print(f"rebuilt triage → {out}", file=sys.stderr)
    return 0


def write_outputs(outdir: Path, cands: list[Cand], report: list[dict], ctx: Ctx, elapsed: float, sources: list[dict]):
    """report.json 最后落盘（先写 .partial 再原子改名）：--reuse 靠它判断「这次抓取已完整写完」，半写的目录不会被复用（2026-11-18 review）。"""
    _write_outputs_inner(outdir, cands, report, ctx, elapsed, sources)
    os.replace(outdir / "report.json.partial", outdir / "report.json")


def _write_outputs_inner(outdir: Path, cands: list[Cand], report: list[dict], ctx: Ctx, elapsed: float, sources: list[dict]):
    outdir.mkdir(parents=True, exist_ok=True)
    try:
        (outdir / "report.json").unlink()
    except FileNotFoundError:
        pass
    with (outdir / "candidates.jsonl").open("w", encoding="utf-8") as f:
        for c in cands:
            f.write(json.dumps(asdict(c), ensure_ascii=False) + "\n")
    meta = {"window_start": iso(ctx.start), "window_end": iso(ctx.end), "generated_at": iso(datetime.now(TZ_LOCAL)),
            "elapsed_sec": round(elapsed, 1), "total_raw": sum(r["n"] for r in report), "total_dedup": len(cands),
            "notes": ctx.notes, "sources": report}
    (outdir / "report.json.partial").write_text(json.dumps(meta, ensure_ascii=False, indent=1), encoding="utf-8")
    # report.md
    by_ch: dict[str, list[dict]] = {}
    for r in report:
        by_ch.setdefault(r["channel"], []).append(r)
    RL = [tr("report_title", start=iso(ctx.start), end=iso(ctx.end)), "",
          tr("report_line", sec=f"{elapsed:.1f}", raw=meta['total_raw'], dd=len(cands)), ""]
    for n in ctx.notes:
        RL.append(f"{tr('note_prefix')}{n}")
    RL.append("")
    RL.append(tr("report_header"))
    RL.append("|---|---|---:|---:|---|")
    for ch in sorted(by_ch):
        for r in sorted(by_ch[ch], key=lambda r: r["id"]):
            RL.append(f"| {ch} | {r['id']} | {r['n']} | {r['sec']}s | {r['status']} |")
    (outdir / "report.md").write_text("\n".join(RL) + "\n", encoding="utf-8")
    if getattr(ctx, "triage_v2", False):
        write_triage_v2(outdir, cands, ctx)
        return
    # triage.md（非论文）+ triage-papers.md（论文）：人工浏览清单
    head = tr("head_a") + "."
    def line(c, summ_len):
        star = {"pushed": "★ ", "seen": "☆ "}.get(c.in_ledger or "", "")
        when = (c.published or "?")[5:16].replace("T", " ")
        extra = f" (+{','.join(c.also_in)})" if c.also_in else ""
        tops = ",".join(c.topics[:4])
        if c.extra.get("arxiv_id") and c.channel == "paper":   # 论文行压缩：只给 arXiv id
            hf = " · HF" if (c.source == "hf_daily" or "hf_daily" in c.also_in) else ""
            t = f"- {star}[{c.pref}] {c.title} — `{c.extra['arxiv_id']}{hf} · {tops}`"
        else:
            t = f"- {star}[{c.pref}] {c.title} — `{c.source}{extra} · {when} · {tops}` {c.url}"
        if summ_len and c.summary:
            t += f"\n  - {c.summary[:summ_len]}"
        return t
    T = [tr("title_nonpaper", start=iso(ctx.start), end=iso(ctx.end)), "", head, ""]
    for ch, label in sec_order():
        items = [c for c in cands if c.channel == ch]
        if not items:
            continue
        items.sort(key=lambda c: (-c.pref, c.published or ""))
        T.append(f"## {label}{i18n.cnt_paren(FL, len(items))}")
        T.append("")
        if getattr(ctx, "compact", False):
            n = 0 if ch in ("repo", "model", "community", "news", "news_zh", "chips_infra") else 120
        else:
            n = 220 if ch not in ("repo", "model", "community") else 140
        T.extend(line(c, n) for c in items)
        T.append("")
    (outdir / "triage.md").write_text("\n".join(T) + "\n", encoding="utf-8")
    papers = [c for c in cands if c.channel == "paper"]
    P = [tr("title_papers", start=iso(ctx.start), end=iso(ctx.end), n=len(papers)), "", head, tr("papers_group_note"), ""]
    groups = [(tr("group_pref3"), lambda c: c.pref >= 3), (tr("group_pref12"), lambda c: 1 <= c.pref <= 2)]
    used = set()
    for label, fn in groups:
        items = [c for c in papers if fn(c) and c.id not in used]
        if not items:
            continue
        items.sort(key=lambda c: (-c.pref, -int(bool(c.also_in)), c.title))
        P.append(f"## {label}{i18n.cnt_paren(FL, len(items))}\n")
        P.extend(line(c, (0 if getattr(ctx, "compact", False) and c.pref < 5 else 220) if c.pref >= 3 else 0) for c in items)
        P.append("")
        used.update(c.id for c in items)
    rest = [c for c in papers if c.id not in used]
    bytopic: dict[str, list[Cand]] = {}
    for c in rest:
        bytopic.setdefault(c.topics[0] if c.topics else tr("other"), []).append(c)
    if rest:
        P.append("## " + tr("rest_plain", n=len(rest)) + "\n")
        for tname in sorted(bytopic, key=lambda k: -len(bytopic[k])):
            P.append(f"### {tname}{i18n.cnt_paren(FL, len(bytopic[tname]))}")
            P.extend(line(c, 0) for c in sorted(bytopic[tname], key=lambda c: c.title))
            P.append("")
    (outdir / "triage-papers.md").write_text("\n".join(P) + "\n", encoding="utf-8")


# ───────────────────────── 主程序 ─────────────────────────
def compute_window(a) -> tuple[datetime, datetime, str]:
    """自然日窗口（本地），左闭右开 [start,end)。
    日报：P = --day 或「昨天」；窗口 = P 00:00 → P+1 00:00；label = P。--start/--end 可单独覆盖一端（过渡期用）。
    周报：Mon..Sun = --day 所在周，默认「上一个完整的周一至周日」；窗口 = Mon 00:00 → 下周一 00:00；label = 周日日期 + "-weekly"。
    只给 --start/--end（无 --daily/--weekly）：任意窗口，label = 窗口最后一刻所在日期。"""
    now = datetime.now(TZ_LOCAL)
    def mid(d):
        return datetime(d.year, d.month, d.day, tzinfo=TZ_LOCAL)
    def pt(x):
        t = datetime.fromisoformat(x)
        return t if t.tzinfo else t.replace(tzinfo=TZ_LOCAL)
    if a.start and a.end and not (a.daily or a.weekly or a.day):
        s, e = pt(a.start), pt(a.end)
        return s, e, (e - timedelta(microseconds=1)).astimezone(TZ_LOCAL).strftime("%Y-%m-%d")
    if a.weekly:
        if a.day:
            d0 = datetime.strptime(a.day, "%Y-%m-%d").date()
        else:
            d0 = now.date() - timedelta(days=7)
        mon = d0 - timedelta(days=d0.weekday())
        s, e = mid(mon), mid(mon + timedelta(days=7))
        label = (mon + timedelta(days=6)).strftime("%Y-%m-%d") + "-weekly"
    else:
        P = datetime.strptime(a.day, "%Y-%m-%d").date() if a.day else now.date() - timedelta(days=1)
        s, e = mid(P), mid(P + timedelta(days=1))
        label = P.strftime("%Y-%m-%d")
    if a.start:
        s = pt(a.start)
    if a.end:
        e = pt(a.end)
    if not s < e:
        raise SystemExit(f"bad window {s} → {e}")
    return s, e, label


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lang", help="language of triage/report labels: en | zh-CN (default: $DIGEST_LANG, else en)")
    ap.add_argument("--daily", action="store_true")
    ap.add_argument("--weekly", action="store_true")
    ap.add_argument("--day", metavar="YYYY-MM-DD", help="daily: the day P to produce (default: yesterday, local time); weekly: the week containing that day (default: last full week)")
    ap.add_argument("--start", help="override the window start (local time; with --daily/--weekly only the start is overridden)")
    ap.add_argument("--end", help="override the window end (exclusive)")
    ap.add_argument("--out")
    ap.add_argument("--only", help="comma-separated source ids")
    ap.add_argument("--skip", help="comma-separated source ids")
    ap.add_argument("--no-state", action="store_true", help="do not read/write state/ (for testing)")
    ap.add_argument("--catchup", action="store_true", help="also collect arXiv batches older than the window that were missed earlier (off by default with natural-day windows)")
    ap.add_argument("--no-catchup", action="store_true", help="(legacy flag, already the default)")
    ap.add_argument("--compact", action="store_true", help="list titles only in triage (default for --weekly)")
    ap.add_argument("--registry", default=str(HERE / "sources.toml"))
    ap.add_argument("--wait-arxiv", type=int, default=0, metavar="MIN", help="minutes to wait for an expected arXiv batch (default 0 = single check)")
    ap.add_argument("--no-fold", action="store_true", help="do not fold the 'rest' tier of papers (default folds off-topic papers without signals; all titles go to triage-papers-rest.md)")
    ap.add_argument("--reuse", action="store_true", help="reuse runs/P/fetch/ if it already holds a complete fetch of the same window made after the window ended (e.g. by the papers prefilter); no network")
    ap.add_argument("--verbose-sources", action="store_true", help="print per-source progress (default: failures only)")
    ap.add_argument("--full-triage", action="store_true", help="use the old full triage (summaries and URLs; daily defaults to the compact one)")
    ap.add_argument("--version", action="version", version=f"ai-digest {__version__}")
    ap.add_argument("--json", action="store_true", help="also print one JSON summary object on stdout when the fetch is done")
    ap.add_argument("--rebuild", metavar="DIR", help="offline: regenerate triage from DIR/candidates.jsonl (--out sets the output dir, default DIR)")
    a = ap.parse_args(argv)
    global FL
    try:
        FL = i18n.resolve_lang(a.lang)
    except ValueError as e:
        ap.error(str(e))
    if a.rebuild:
        return rebuild_triage(Path(a.rebuild), Path(a.out) if a.out else None, a)

    reg = tomllib.loads(Path(a.registry).read_text(encoding="utf-8"))
    settings = reg.get("settings", {})
    sources = [s for s in reg["source"] if s.get("enabled", True)]
    if a.only:
        keep = set(a.only.split(","))
        sources = [s for s in sources if s["id"] in keep]
    if a.skip:
        drop = set(a.skip.split(","))
        sources = [s for s in sources if s["id"] not in drop]
    start, end, label = compute_window(a)
    state_path = _state_dir() / "state.json"
    state = {}
    if not a.no_state and state_path.exists():
        state = json.loads(state_path.read_text(encoding="utf-8"))
    ctx = Ctx(settings, start, end, state, not a.no_state, a.catchup and not a.no_catchup, os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN"))
    ctx.compact = a.compact or a.weekly
    ctx.wait_arxiv_minutes = a.wait_arxiv
    ctx.weekly = a.weekly
    ctx.custom_window = bool(a.start and a.end and not (a.daily or a.weekly or a.day))
    ctx.triage_v2 = not (a.weekly or a.full_triage or a.compact)
    ctx.label_day = label
    ctx.fold_rest = not a.no_fold
    ctx.only_mode = bool(a.only)
    ctx.seen_prior = load_triage_seen() if ctx.triage_v2 else {}
    # --only 的补查输出写到单独目录，不覆盖整份 fetch/（2026-11-17 事故：--only arxiv 重跑把当天 candidates/triage 清空了）
    default_dir = "fetch" if not a.only else "fetch-extra"
    root = _work_root()
    outdir = Path(a.out) if a.out else root / "runs" / label / default_dir
    # 同一天目录的完整抓取串行化（00:10 论文初筛与 00:30 日报若同时跑，后到的等前一个写完再判断能否复用；2026-11-18 review）
    _lockf = None
    try:
        if not a.only and not a.out:
            import fcntl
            outdir.parent.mkdir(parents=True, exist_ok=True)
            _lockf = open(outdir.parent / ".fetch.lock", "w", encoding="utf-8")
            t_wait = time.time()
            while True:
                try:
                    fcntl.flock(_lockf, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.time() - t_wait > 900:
                        raise SystemExit(tr("fetch_lock_timeout", outdir=outdir))
                    if time.time() - t_wait < 1:
                        print(tr("fetch_lock_wait", outdir=outdir), file=sys.stderr)
                    time.sleep(5)
        if a.reuse and not a.only and not a.out:
            rj = outdir / "report.json"
            try:
                meta0 = json.loads(rj.read_text(encoding="utf-8"))
                fresh = (meta0.get("window_start") == iso(start) and meta0.get("window_end") == iso(end)
                         and datetime.fromisoformat(meta0["generated_at"]) >= end and (outdir / "candidates.jsonl").exists()
                         and (outdir / "triage.md").exists())
            except Exception:
                fresh = False
            if fresh:
                print(tr("fetch_reuse", outdir=outdir, at=meta0["generated_at"]), file=sys.stderr)
                for n in meta0.get("notes", []):
                    print(f"{tr('note_prefix')}{n}", file=sys.stderr)
                errs0 = [r for r in meta0.get("sources", []) if r.get("status") != "ok"]
                for e in errs0:
                    print(f"  ! {e['id']}: {e['status']}", file=sys.stderr)
                if a.json:
                    print(json.dumps({
                        "window": [meta0.get("window_start"), meta0.get("window_end")],
                        "sources": len(meta0.get("sources", [])),
                        "raw": meta0.get("total_raw", 0),
                        "dedup": meta0.get("total_dedup", 0),
                        "errors": [{"id": e["id"], "status": e["status"]} for e in errs0],
                        "out": str(outdir),
                        "reused": True,
                        "files": {n: str(outdir / n) for n in ("candidates.jsonl", "triage.md", "report.json", "report.md")},
                    }, ensure_ascii=False))
                return 0
        # 同一天目录已有一次完整抓取（例：过渡期 11/17 的第二次运行、手动重跑）→ 先把旧的整份挪走，绝不覆盖
        if not a.out and not a.only and (outdir / "candidates.jsonl").exists():
            mt = datetime.fromtimestamp((outdir / "candidates.jsonl").stat().st_mtime, TZ_LOCAL)
            arch = outdir.with_name(f"fetch-prev-{mt:%m%d-%H%M}")
            n = 2
            while arch.exists():
                arch = outdir.with_name(f"fetch-prev-{mt:%m%d-%H%M}-{n}"); n += 1
            outdir.rename(arch)
            print(f"archived previous fetch → {arch}", file=sys.stderr)
        print(f"window {iso(start)} → {iso(end)}  sources={len(sources)}  out={outdir}", file=sys.stderr)

        t0 = time.time()
        report, allc = [], []

        def run(src):
            t = time.time()
            try:
                items = FETCHERS[src["type"]](ctx, src)
                st = "ok"
            except Exception as e:
                items, st = [], f"ERROR {type(e).__name__}: {str(e)[:160]}"
            return src, items, st, round(time.time() - t, 1)

        with cf.ThreadPoolExecutor(settings.get("workers", 24)) as ex:
            futs = [ex.submit(run, s) for s in sources]
            for fu in cf.as_completed(futs):
                src, items, st, sec = fu.result()
                report.append({"id": src["id"], "name": src.get("name"), "channel": src["channel"], "type": src["type"], "n": len(items), "sec": sec, "status": st})
                allc.extend(items)
                if a.verbose_sources or st != "ok":   # 默认只打印失败源（153 行进度表约 2K token，且 report.md 里已有）
                    print(f"  {src['id']:<24} {len(items):>4}  {sec:>5}s  {st}", file=sys.stderr)

        ledger = load_ledger_keys()
        for c in allc:
            tag(c)
        cands = dedupe(allc)
        for c in cands:
            k = f"arxiv:{c.extra['arxiv_id']}" if c.extra.get("arxiv_id") else canon_url(c.url)
            c.in_ledger = ledger.get(k) or ledger.get("t:" + norm_title(c.title)) or False
        elapsed = time.time() - t0
        write_outputs(outdir, cands, report, ctx, elapsed, sources)
        if ctx.triage_v2 and not a.no_state and not a.only:
            save_triage_seen(cands, label, ctx.seen_prior)
        if not a.no_state:
            state_path.parent.mkdir(parents=True, exist_ok=True)
            state_path.write_text(json.dumps(ctx.state, ensure_ascii=False, indent=0), encoding="utf-8")
        errs = [r for r in report if r["status"] != "ok"]
        print(f"done in {elapsed:.1f}s: raw={len(allc)} dedup={len(cands)} errors={len(errs)} → {outdir}/candidates.jsonl", file=sys.stderr)
        for e in errs:
            print(f"  ! {e['id']}: {e['status']}", file=sys.stderr)
        # 把 report.md 里的「注：」行（arXiv 批次、HF 日榜、失败的 org 等）直接打出来，agent 不必再读 report.md
        try:
            for ln in (outdir / "report.md").read_text(encoding="utf-8").splitlines():
                if ln.startswith(tr("note_prefix")):
                    print(ln, file=sys.stderr)
        except Exception:
            pass
        if a.json:
            print(json.dumps({
                "window": [iso(start), iso(end)], "sources": len(sources), "raw": len(allc), "dedup": len(cands),
                "errors": [{"id": e["id"], "status": e["status"]} for e in errs], "out": str(outdir),
                "files": {n: str(outdir / n) for n in ("candidates.jsonl", "triage.md", "report.json", "report.md")},
            }, ensure_ascii=False))
        return 0
    finally:
        ctx.close()
        if _lockf is not None:
            _lockf.close()


if __name__ == "__main__":
    sys.exit(main())
