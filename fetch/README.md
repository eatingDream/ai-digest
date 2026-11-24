# AI Digest Recall Layer (`fetch/`)

English | [简体中文](#ai精选-召回层fetch)

Pull ~153 machine-readable open sources in parallel and write a deduplicated candidate shortlist. Scheduled curation agents **always run this step first**, followed by optional capability-gated X/Twitter search and primary-source verification.

```bash
python3 fetch/fetch.py --daily     # daily (00:30 local): P = yesterday; window = [P 00:00, P+1 00:00)
python3 fetch/fetch.py --daily --day 2026-11-16                # specific day P
python3 fetch/fetch.py --daily --day 2026-11-17 --start 2026-11-17T09:00   # override start only
python3 fetch/fetch.py --weekly    # weekly (Mon 01:30): last Mon 00:00 -> this Mon 00:00; --day X = week of X
python3 fetch/fetch.py --start 2026-11-14T09:00 --end 2026-11-15T09:00 --out /tmp/x   # custom window
```

Dependencies: Python 3.11+ standard library + `requests` + `lxml`. **No API keys required**; if `GITHUB_TOKEN` or `GH_TOKEN` is set in the environment, `fetch.py` passes `Authorization: Bearer <token>` automatically on GitHub REST API calls (raising the rate limit from 60 to 5,000 req/hr) and emits an explicit warning on HTTP 403/429. Typical runtime: 10–40 seconds (~30 seconds on weekdays with arXiv).

## Outputs (`<DIGEST_ROOT>/runs/<P>[-weekly]/fetch/` by default; prior complete fetches are archived to `fetch-prev-MMDD-HHMM/`)

| File | Purpose |
|---|---|
| `triage.md` | **Read first**: all non-paper candidates grouped by channel (lab official / models / tool releases / GitHub / essays / podcasts / chips & infra / policy / news / Chinese media / community), ranked within each channel by `fetch/preferences.json` weights |
| `triage-papers.md` | Papers: weight ≥3 includes abstracts, weights 1–2 list titles, off-topic papers with signals list titles and the rest are folded into `triage-papers-rest.md`. **Scan all tiers** — off-topic breakthroughs still count |
| `candidates.jsonl` | One deduplicated candidate per line (full summary, authors, topics, source, `also_in`, `in_ledger`) — inspect via `fetch/cand.py`, never read whole file into context |
| `report.md` / `report.json` | Per-source item counts, latency, errors, and `notes` (arXiv batch arrivals, HF daily boards, GitHub rate-limit warnings) |

Candidate fields: `title url source channel kind published(local ISO) summary authors topics pref also_in in_ledger extra`.
- `pref` is a ranking weight only (see `preferences.example.json`), **never an inclusion threshold**.
- `in_ledger`: `"pushed"` = already published in `ledger.json` (★); `"seen"` = reviewed, not pushed (☆). Re-include pushed items only on substantive new developments.
- `extra.discovery_only=true` (HF/GitHub trending, GitHub search): discovery entry point only — popularity never justifies inclusion.
- `extra.date_unknown=true`: HTML diff discovery without an RSS timestamp — open the primary page to verify publication date.
- Popularity metrics (stars, likes, downloads, HN points) are never written into outputs.

## Sources

Registry: `fetch/sources.toml` (`id`, `type`, `url`, `channel`, `kind`, `filter`). Edit only this file to add or disable feeds.

| Channel | Coverage |
|---|---|
| `paper` | arXiv 12 categories (`cs.LG/CL/AI/CV/MA/SE/RO/CR/DC/HC/IR`, `stat.ML`, new + cross), HF Daily Papers, Nature ML, Science |
| `model` | ~67 lab/org Hugging Face model feeds, HF trending models/datasets/Spaces (recent creations only; filters quantized/merged/abliterated derivatives) |
| `repo` | GitHub Trending (`all`/`python`/`ts`/`rust`, AI-filtered), ~28 lab org new repos, GitHub 3-day new-repo search (spam-filtered) |
| `tooling` | Official stable releases across ~67 core repositories (skips `alpha`/`beta`/`rc`/`dev`, keeps latest per window), engineering blogs (Cursor, Windsurf, Ollama, PyTorch, vLLM, LangChain, GitHub, Cloudflare, Replit, Zed) |
| `lab` | OpenAI, Anthropic (news/research/engineering/alignment/circuits), DeepMind, Google Research/Keyword/Developers, Meta AI/Engineering, MSR, NVIDIA, Apple ML, Mistral, xAI, DeepSeek, MiniMax, Thinking Machines, Cohere, Ai2, Together, HF, AWS ML, Databricks, Sakana, BAIR, Figure, Cognition |
| `research_blog` / `newsletter` | Lilian Weng, Simon Willison, Interconnects, Import AI, Latent Space, The Batch, Chip Huyen, Raschka, Karpathy, Epoch, METR, Zvi, Mollick, Eugene Yan, Hamel, Phil Schmid, Grootendorst, Answer.AI, Alignment Forum, LessWrong, Stratechery, ChinAI, ChinaTalk, Last Week in AI, Understanding AI, Exponential View, etc. |
| `podcast` | Dwarkesh, Latent Space, No Priors, Cognitive Revolution, TWIML, Lex Fridman; YouTube channels (DeepMind, Anthropic, Dwarkesh, Yannic Kilcher) |
| `chips_infra` | SemiAnalysis, Next Platform, HPCwire, DCD, ServeTheHome, Tom's Hardware, EE Times, DIGITIMES, Chips and Cheese, Fabricated Knowledge |
| `news` | The Verge, TechCrunch, Ars Technica, MIT Tech Review, WIRED, The Information, Bloomberg, FT, CNBC, Semafor, The Decoder, MarkTechPost, SiliconANGLE, 404 Media, Rest of World, IEEE Spectrum, Robot Report, MIT News, Crunchbase, SCMP, Pandaily, TechNode |
| `news_zh` | QbitAI (量子位), 36Kr, InfoQ CN, Leiphone, ITHome, GeekPark, TMTPost, sspai |
| `policy` | CAC, Federal Register, NIST, UK AISI, EU Digital Strategy, CSET, Hyperdimensional, Transformer, CAIS |
| `community` | Hacker News (AI-filtered), Reddit (`r/MachineLearning` + `r/LocalLLaMA` + `r/singularity` combined feed), Lobsters |

Optional X/Twitter recall is documented in `x-recall.md` (Chinese: `x-recall.zh-CN.md`). If the agent runner lacks X search tools, prompts cleanly skip X recall without failing; unreplaced `<account_1>` / `<zh_account_1>` watch-circle queries are also skipped automatically.

## Windows & arXiv Calendar (Natural-Day Windows)

- Windows are half-open `[start, end)` in local time (`DIGEST_TZ`, default `UTC`). Daily = previous local calendar day; Weekly = last Monday `00:00` → this Monday `00:00`.
- **arXiv batch `D` is announced at 20:00 US Eastern on `D-1` (Sun–Thu nights)**, falling inside weekday `P` windows well before the `00:30` daily run. Weekends have no new arXiv batches.
- Missing weekday arXiv batches raise an explicit `⚠️ weekday batch … is missing` note after an automatic retry (see `troubleshoot-arxiv.md`).
- **HF Daily Papers**: fetches completed UTC boards not yet recorded in `state.json` and deduplicates against arXiv by arXiv ID (`also_in` includes `hf_daily`).

## Token-Saving Helper CLIs

- `fetch.py`: prints only failed sources + `Note:` lines by default (`--verbose-sources` shows all); folds off-topic unsignaled papers (`--no-fold` disables); `--reuse` reuses an existing complete window fetch; serializes concurrent runs via `.fetch.lock`.
- `page.py`: parallel slim primary-source reader (`page.py URL1 "URL2@@kw1,kw2" URL3`) with public-IP SSRF protection.
- `xparse.py`: deduplicates X search JSON dumps, strips engagement counts, flags `next_token` truncation.
- `ledger.py`: targeted `find` / `recent` lookups (`exit 0` with empty results when `ledger.json` does not exist yet) and atomic `add` (auto-initializes `ledger.json` and `logs/ledger-backup/` on first write, optionally appending `filter-log.md`).
- `weekly_late.py` / `cand.py --since YYYY-MM-DD`: offline multi-day lookup and "late signal, never pushed" shortlist for weekly issues.
- `check_cands.py`: validates and atomically renames `runs/P/paper-cands.md` from the papers prefilter job.

## Configuration

- `DIGEST_TZ`: IANA time zone defining natural-day boundaries (default `UTC`).
- `DIGEST_ROOT`: isolated working directory for `runs/`, `ledger.json`, `logs/ledger-backup/`, and `state/{state,triage-seen}.json` (defaults to repo root / `fetch/state/`).
- `LEDGER_PATH`: explicit `ledger.json` location override.
- `fetch/preferences.json` (or `DIGEST_PREFERENCES_FILE`): topic ranking weights (see `preferences.example.json`).
- `GITHUB_TOKEN` / `GH_TOKEN` (optional): raises GitHub REST API rate limits from 60 to 5,000 req/hr.
- `--lang en|zh-CN` / `DIGEST_LANG`: language of triage/report/CLI labels (default `en`).

---

# AI精选 召回层（fetch）

[English](#ai-digest-recall-layer-fetch) | 简体中文

一条命令并行拉取约 153 个机器可读公开信源，输出去重后的候选清单。定时 agent 每次**先跑它**，再按运行器能力可选执行 X 检索与一手出处核验。

```bash
python3 fetch/fetch.py --daily     # 日报（凌晨 00:30 跑）：P = 昨天（本地）；窗口 = P 00:00 → P+1 00:00，左闭右开 [start,end)
python3 fetch/fetch.py --daily --day 2026-11-16                # 指定 P
python3 fetch/fetch.py --daily --day 2026-11-17 --start 2026-11-17T09:00   # 覆盖窗口起点
python3 fetch/fetch.py --weekly    # 周报（周一凌晨跑）：上周一 00:00 → 本周一 00:00；--day X = X 所在那一周；--start 覆盖起点
python3 fetch/fetch.py --start 2026-11-14T09:00 --end 2026-11-15T09:00 --out /tmp/x   # 任意窗口
```

依赖：Python 3.11+ 标准库 + `requests` + `lxml`。**无需任何密钥**；如果环境里有 `GITHUB_TOKEN` / `GH_TOKEN`，`fetch.py` 会自动加 `Authorization: Bearer <token>` 请求头（将 GitHub API 限额从 60 次/小时提升至 5,000 次/小时），遇到 HTTP 403/429 时输出明确的限流提示。典型耗时 10–40 秒（工作日含 arXiv 约 30 秒）。

## 输出（默认 `<DIGEST_ROOT>/runs/<P>[-weekly]/fetch/`；周报目录以周日日期命名；重跑时旧目录自动归档到 `fetch-prev-MMDD-HHMM/`）

| 文件 | 用途 |
|---|---|
| `triage.md` | **先读这个**：论文以外的全部候选，按版面分组（实验室官方 / 模型 / 工具发布 / GitHub / 长文 / 播客 / 算力 / 政策 / 新闻 / 中文 / 社区），每组按偏好权重排序（`fetch/preferences.json`） |
| `triage-papers.md` | 论文：偏好权重 ≥3 带摘要，1–2 只列标题，其余有信号列标题、无信号折叠进 `triage-papers-rest.md`。**全部扫一遍**，方向外的重大结果也要收 |
| `candidates.jsonl` | 一行一个候选（完整摘要、作者、主题、来源、`also_in`、`in_ledger`），用 `fetch/cand.py` 按需查询，切勿整份读入上下文 |
| `report.md` / `report.json` | 每个信源的条数、耗时、错误；`notes` 里有 arXiv 批次说明、HF 榜单与 GitHub 限流警告 |

候选字段：`title url source channel kind published(本地 ISO) summary authors topics pref also_in in_ledger extra`。
- `pref` 只是排序用的偏好权重（见 `preferences.example.json`），**不是门槛**。
- `in_ledger`：`"pushed"` = `ledger.json` 里已推过（★），`"seen"` = 看过未推（☆）；已推的只有出现实质新进展才再写，看过未推的可按新证据重新判断。
- `extra.discovery_only=true`（HF/GitHub trending、GitHub 搜索）：只是发现入口，热度不得作为纳入理由。
- `extra.date_unknown=true`：来自无 RSS 的网页差分（首次出现于本窗口），需打开原文核对日期。
- 热度数字（星标、点赞、下载、HN 分数）一律不写入输出。

## 信源

注册表：`sources.toml`（每条有 `id` / `type` / `url` / `channel` / `kind` / `filter`）。改信源只改这个文件。

| 频道 | 内容 |
|---|---|
| paper | arXiv 12 个分类 new+cross（cs.LG/CL/AI/CV/MA/SE/RO/CR/DC/HC/IR、stat.ML）、HF Daily Papers、Nature ML、Science |
| model | HF 约 67 个机构的新模型、HF trending 模型/数据集/Spaces（只看近 2–3 周新建的，过滤量化/合并/去审查等衍生物） |
| repo | GitHub Trending（all/python/ts/rust，AI 过滤）、约 28 个实验室组织的新仓库、GitHub 搜索近 3 天新建仓库（去刷星垃圾） |
| tooling | 约 67 个关键仓库的正式 Release（跳过 alpha/beta/rc/dev，同仓库只留最新）、Cursor/Windsurf/Ollama/PyTorch/vLLM/LangChain/GitHub/Cloudflare/Replit/Zed 博客 |
| lab | OpenAI、Anthropic（新闻/研究/工程/红队/对齐/Transformer Circuits）、DeepMind、Google Research/Keyword/Developers、Meta AI/Engineering、MSR、NVIDIA×2、Apple ML、Mistral、xAI、DeepSeek、MiniMax、Thinking Machines、Cohere、Ai2、Together、HF、AWS ML、Databricks、Sakana、BAIR、Figure、Cognition |
| research_blog / newsletter | Lilian Weng、Simon Willison、Interconnects、Import AI、Latent Space、The Batch、Chip Huyen、Raschka、Karpathy、Epoch、METR、Zvi、Normal Technology、Mollick、Gary Marcus、Eugene Yan、Hamel、Phil Schmid、Grootendorst、Answer.AI、Alignment Forum、LessWrong、Stratechery、ChinAI、ChinaTalk、Last Week in AI、Understanding AI、Exponential View、Big Technology、Platformer、Not Boring、O'Reilly Radar |
| podcast | Dwarkesh、Latent Space、No Priors、Cognitive Revolution、TWIML、Lex Fridman；YouTube：DeepMind、Anthropic、Dwarkesh、Yannic Kilcher |
| chips_infra | SemiAnalysis、Next Platform、HPCwire、DCD、ServeTheHome、Tom's Hardware、EE Times、DIGITIMES、Chips and Cheese、Fabricated Knowledge |
| news | Verge、TechCrunch、Ars、MIT TR、WIRED、The Information、Bloomberg、FT、CNBC、Semafor、The Decoder、MarkTechPost、SiliconANGLE、404 Media、Rest of World、IEEE Spectrum×2、Robot Report、MIT News、Crunchbase、SCMP、Pandaily、TechNode |
| news_zh | 量子位、36氪、InfoQ 中文、雷峰网、IT之家、极客公园、钛媒体、少数派 |
| policy | 网信办、Federal Register、NIST、UK AISI（gov.uk + 博客）、EU Digital Strategy、CSET、Hyperdimensional、Transformer、CAIS |
| community | Hacker News（AI 过滤）、Reddit（r/MachineLearning + r/LocalLLaMA + r/singularity 合并 1 次请求）、Lobsters |

X 检索为可选增强（见 `x-recall.zh-CN.md` / 英文 `x-recall.md`）：当运行器没有提供 X 只读检索工具时会自动干净跳过；未替换 `<account_1>` / `<zh_account_1>` 占位符的个人关注组也会自动跳过。

## 时间与 arXiv 日历（自然日窗口）

- 窗口是**左闭右开** `[start,end)`，按每条的发布时间戳筛选（本地时间）。日报 = 前一个本地自然日；周报 = 上周一 00:00 → 本周一 00:00。
- **arXiv 列表日期 D 在 D 前一天 20:00 美东公开**（周日至周四晚）= 本地周一至周五 09:00（冬令时）/ 08:00（夏令时），一定落在 P 日窗口内；凌晨 00:30 跑时早已上线，**不等待、不补收**（`--catchup` 默认关）。本地周六、周日没有 arXiv 新批次。
- 工作日 P 没有批次会触发内部重试并输出「⚠️ 工作日批次 … 在 /pastweek 与 /new 上都没有」警告（处理流程见 `troubleshoot-arxiv.zh-CN.md`）。
- HF Daily：按 `state.json` 增量拉取已完整收榜的 UTC 日榜单，并在 `dedupe` 里按 arXiv id 与 arXiv 合并（`also_in` 含 `hf_daily`）。

## 省 token 的小工具

- `fetch.py`：默认只打印失败源 + `注：` 行（`--verbose-sources` 恢复逐源输出）；论文「其余」档折叠（`--no-fold` 关闭，全量标题写 `triage-papers-rest.md`）；`--reuse` 复用同一窗口已抓过的结果；带 `.fetch.lock` 文件锁。
- `page.py`：支持一次传多个 URL 并行瘦身抓取（`page.py URL1 "URL2@@关键词a,关键词b" URL3`），内置公网 IP SSRF 防护。
- `xparse.py`：跨组去重、跳过转推、去互动数字；带 `next_token` 时提示「⚠️ 截断」。
- `ledger.py`：`ledger.json` 尚不存在时 `find` / `recent` 干净返回空结果（退出码 0），首次 `add` 自动创建 `ledger.json` 及父目录与 `logs/ledger-backup/`，并可通过 `--log runs/P/filter-log.md` 同步追加筛选日志。
- `weekly_late.py` / `cand.py --since YYYY-MM-DD`：周报离线查询多日目录与生成「后来才有信号、未推过」清单。
- `check_cands.py`：校验并原子重命名论文初筛产物 `runs/P/paper-cands.md`。

## 配置

- `DIGEST_TZ`：本地时区（IANA 名，默认 `UTC`），决定「自然日」窗口和显示时间。
- `DIGEST_ROOT`：工作目录隔离（设置后 `runs/`、`ledger.json`、`logs/ledger-backup/` 与 `state/{state,triage-seen}.json` 全部写入 `DIGEST_ROOT` 下，不污染仓库根目录）。
- `LEDGER_PATH`：单独覆盖 `ledger.json` 路径。
- `fetch/preferences.json`（或 `DIGEST_PREFERENCES_FILE`）：主题偏好权重，格式见 `preferences.example.json`；只用于排序，不是门槛。
- `GITHUB_TOKEN` / `GH_TOKEN`（可选）：将 GitHub API 限额从 60 次/小时提升至 5,000 次/小时。
- `--lang en|zh-CN` / `DIGEST_LANG`：决定 triage / report / CLI 输出标签语言（默认 `en`）。
