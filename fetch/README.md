# AI Digest Recall Layer (`fetch/`)

English | [简体中文](#ai精选-召回层fetch)

Pull ~155 read-only open sources from [`sources.toml`](sources.toml) in parallel into deduplicated candidate shortlists using the single `ai-digest` Rust binary. Curation agents **always run this step first** (zero API keys required; optional `GITHUB_TOKEN`/`GH_TOKEN` raises GitHub REST API limits from 60 to 5,000 req/hr and warns on HTTP 403/429).

```bash
ai-digest fetch --daily                                                # yesterday [P 00:00, P+1 00:00)
ai-digest fetch --daily --day 2026-11-16                               # specific day P
ai-digest fetch --daily --day 2026-11-17 --start 2026-11-17T09:00      # override window start
ai-digest fetch --weekly                                               # last Mon 00:00 -> this Mon 00:00 (--day X)
ai-digest fetch --start 2026-11-14T09:00 --end 2026-11-15T09:00 --out /tmp/x
```

## Outputs (`<DIGEST_ROOT>/runs/<P>[-weekly]/fetch/`; prior complete fetches archive to `fetch-prev-MMDD-HHMM/`)

| File | Purpose |
|---|---|
| `triage.md` | **Read first**: all non-paper candidates grouped by channel, ranked within each channel by `fetch/preferences.json` weights |
| `triage-papers.md` | Papers: weight ≥3 with abstracts, 1–2 titles only, signaled off-topic titles, rest folded into `triage-papers-rest.md` (**scan all tiers**) |
| `candidates.jsonl` | One deduplicated candidate per line (full metadata, `also_in`, `in_ledger`) — query via `ai-digest cand`, never load whole file |
| `report.md` / `report.json` | Per-source counts, latency, errors, and `notes` (arXiv batch arrivals, HF daily boards, GitHub rate-limit warnings) |

- Candidate fields: `title url source channel kind published(local ISO) summary authors topics pref also_in in_ledger extra`. Popularity metrics (stars, likes, downloads, HN points) are never written.
- `pref` is a ranking weight only (never an inclusion threshold); `in_ledger` marks `"pushed"` (★, re-include only on substantive new developments) or `"seen"` (☆).
- `extra.discovery_only=true` (trending/search) is discovery-only; `extra.date_unknown=true` (HTML diff) requires checking the primary page date.

## Sources & Windows

- **Registry**: edit only [`sources.toml`](sources.toml) (`id`, `type`, `url`, `channel`, `kind`, `filter`) to add or disable feeds. Channels cover `lab` (official AI labs), `model` (HF org models & trending, filtering derivatives), `paper` (arXiv 12 categories, HF Daily, Nature ML, Science), `repo` (GitHub trending, org repos, new-repo search), `tooling` (core stable releases & engineering blogs), `research_blog` / `newsletter`, `podcast` (incl. YouTube), `chips_infra`, `news`, `news_zh`, `policy`, and `community` (HN, Reddit, Lobsters). Optional read-only X recall lives in [`x-recall.md`](x-recall.md) — including both **free** (open-feeds-only, read-only web search grounding, Bluesky/RSS bridges) and **paid** (xAI `x_search` API, X API v2 Bearer token, third-party read-only search APIs) paths without a Grok Bot.
- **Windows & arXiv**: half-open `[start, end)` in `DIGEST_TZ`. arXiv batch `D` announces at 20:00 US Eastern on `D-1` (Sun–Thu; no weekend batches); missing weekday batches trigger an automatic retry + warning (see [`troubleshoot-arxiv.md`](troubleshoot-arxiv.md)). HF Daily fetches completed UTC boards via `state.json` and merges with arXiv by ID (`also_in` includes `hf_daily`).

## Helper Subcommands

- `ai-digest fetch`: prints only failed sources + `Note:` lines by default (`--verbose-sources` for all); folds unsignaled off-topic papers (`--no-fold`); `--reuse` reuses an existing complete fetch; serializes runs via `.fetch.lock`.
- `ai-digest cand`: targeted lookup in `candidates.jsonl` (`--since YYYY-MM-DD` for multi-day weekly lookups).
- `ai-digest page`: parallel slim primary-source reader (`ai-digest page URL1 "URL2@@kw1,kw2" URL3`) with DNS-pinned public-IP SSRF protection.
- `ai-digest xparse`: deduplicates X search JSON dumps, skips retweets, strips engagement counts, flags `next_token` truncation.
- `ai-digest weekly-late`: offline "late signal, never pushed" shortlist for weekly issues.
- `ai-digest check-cands`: validates and atomically renames `runs/P/paper-cands.md` from the papers prefilter.
- `ai-digest ledger`: targeted `find` / `recent` (clean exit 0 before `ledger.json` exists) and atomic `add` (auto-initializes `ledger.json` + `logs/ledger-backup/`, optional `--log`).

Environment variables (`DIGEST_TZ`, `DIGEST_ROOT`, `LEDGER_PATH`, `DIGEST_PREFERENCES_FILE`, `DIGEST_LANG`, `GITHUB_TOKEN`): see root [README](../README.md#configuration).

---

# AI精选 召回层（fetch）

[English](#ai-digest-recall-layer-fetch) | 简体中文

使用单文件 `ai-digest` Rust 二进制并行拉取 [`sources.toml`](sources.toml) 中约 155 个机器可读公开信源，输出去重候选清单。定时精选 agent **始终先跑此步**（零密钥、无需 Python 解释器；可选 `GITHUB_TOKEN`/`GH_TOKEN` 将 GitHub API 限额从 60 提至 5,000 次/小时，并在 403/429 时明确告警）。

```bash
ai-digest fetch --daily                                                # 昨天 [P 00:00, P+1 00:00)
ai-digest fetch --daily --day 2026-11-16                               # 指定日期 P
ai-digest fetch --daily --day 2026-11-17 --start 2026-11-17T09:00      # 覆盖窗口起点
ai-digest fetch --weekly                                               # 上周一 00:00 → 本周一 00:00（--day X）
ai-digest fetch --start 2026-11-14T09:00 --end 2026-11-15T09:00 --out /tmp/x
```

## 输出（默认 `<DIGEST_ROOT>/runs/<P>[-weekly]/fetch/`；重跑时旧完整目录归档到 `fetch-prev-MMDD-HHMM/`）

| 文件 | 用途 |
|---|---|
| `triage.md` | **先读这个**：论文外全部候选，按频道分组并依 `fetch/preferences.json` 权重排序 |
| `triage-papers.md` | 论文：权重 ≥3 带摘要，1–2 列标题，方向外有信号列标题，其余折叠到 `triage-papers-rest.md`（**全部扫一遍**） |
| `candidates.jsonl` | 一行一个去重候选（完整元数据、`also_in`、`in_ledger`），用 `ai-digest cand` 查询，勿整份读入上下文 |
| `report.md` / `report.json` | 各信源条数、耗时、错误及 `notes`（arXiv 批次说明、HF 榜单、GitHub 限流告警） |

- 候选字段：`title url source channel kind published(本地 ISO) summary authors topics pref also_in in_ledger extra`。热度数字（星标、点赞、下载、HN 分数）一律不写入。
- `pref` 仅作排序权重（不是门槛）；`in_ledger` 标记 `"pushed"`（★，仅在实质新进展时复推）或 `"seen"`（☆）。
- `extra.discovery_only=true`（趋势/搜索）仅作发现入口；`extra.date_unknown=true`（网页差分）需打开原文核对日期。

## 信源与窗口

- **注册表**：增删信源只改 [`sources.toml`](sources.toml)（`id` / `type` / `url` / `channel` / `kind` / `filter`）。覆盖 13 个频道：`lab`（AI 实验室官方）、`model`（HF 机构与 trending，过滤衍生物）、`paper`（arXiv 12 分类、HF Daily、Nature ML、Science）、`repo`（GitHub 趋势/机构/新仓搜索）、`tooling`（核心仓库正式 Release 与工程博客）、`research_blog` / `newsletter`、`podcast`（含 YouTube）、`chips_infra`、`news`、`news_zh`、`policy` 及 `community`（HN、Reddit、Lobsters）。可选 X 只读检索见 [`x-recall.zh-CN.md`](x-recall.zh-CN.md)——附无 Grok Bot 时的安全免费途径（纯公开源、只读网页检索、Bluesky/RSS 桥接）与付费途径（xAI `x_search` 官方 API、X API v2 只读 Bearer Token、第三方只读搜索 API）。
- **时间与 arXiv**：按 `DIGEST_TZ` 划定左闭右开 `[start, end)` 自然日窗口（日报=昨天，周报=上周一 00:00 → 本周一 00:00）。arXiv 批次 `D` 于 `D-1` 美东 20:00 公开（周日至周四晚，周末无新批次）；工作日缺批次会自动重试并告警（见 [`troubleshoot-arxiv.zh-CN.md`](troubleshoot-arxiv.zh-CN.md)）。HF Daily 依 `state.json` 增量拉取完整 UTC 日榜并按 arXiv ID 合并（`also_in` 含 `hf_daily`）。

## 辅助子命令

- `ai-digest fetch`：默认只打印失败源与 `注：` 行（`--verbose-sources` 全量输出）；默认折叠无信号方向外论文（`--no-fold` 关闭）；`--reuse` 复用完整窗口；`.fetch.lock` 防并发冲突。
- `ai-digest cand`：按需查询 `candidates.jsonl`（周报可用 `--since YYYY-MM-DD` 跨多日查询）。
- `ai-digest page`：并行瘦身读取一手原文（`ai-digest page URL1 "URL2@@关键词a,关键词b" URL3`），内置 DNS 绑定与公网 IP SSRF 防护。
- `ai-digest xparse`：跨组去重、跳过转推、去除互动数字，并在出现 `next_token` 时提示截断。
- `ai-digest weekly-late`：离线生成周报「后来有信号但未推过」候选清单。
- `ai-digest check-cands`：校验并原子重命名论文初筛产物 `runs/P/paper-cands.md`。
- `ai-digest ledger`：`find` / `recent`（`ledger.json` 不存在时返回空结果，退出码 0）与原子 `add`（自动初始化 `ledger.json` 及 `logs/ledger-backup/`，支持 `--log`）。

环境变量（`DIGEST_TZ`、`DIGEST_ROOT`、`LEDGER_PATH`、`DIGEST_PREFERENCES_FILE`、`DIGEST_LANG`、`GITHUB_TOKEN`）详见根目录 [README.zh-CN.md](../README.zh-CN.md#配置)。
