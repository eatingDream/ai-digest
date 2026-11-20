# AI精选 召回层（fetch）

一条命令并行拉取 ~150 个机器可读信源，输出去重后的候选清单。定时 agent 每次**先跑它**，再做 X 检索和人工判断。

```bash
python3 fetch/fetch.py --daily     # 日报（凌晨 00:30 跑）：P = 昨天（本地）；窗口 = P 00:00 → P+1 00:00，左闭右开 [start,end)
python3 fetch/fetch.py --daily --day 2026-11-16                # 指定 P
python3 fetch/fetch.py --daily --day 2026-11-17 --start 2026-11-17T09:00   # 覆盖窗口起点
python3 fetch/fetch.py --weekly    # 周报（周一凌晨跑）：上周一 00:00 → 本周一 00:00；--day X = X 所在那一周；--start 覆盖起点
python3 fetch/fetch.py --start 2026-11-14T09:00 --end 2026-11-15T09:00 --out /tmp/x   # 任意窗口
```

依赖：Python 3.11+ 标准库 + `requests` + `lxml`。无需任何密钥；如果环境里有 `GITHUB_TOKEN`/`GH_TOKEN` 会自动用于 GitHub API（提高限额），没有也能跑。
典型耗时 10–40 秒（工作日含 arXiv 约 30 秒）。

## 输出（默认 `runs/<P>[-weekly]/fetch/`；周报目录以周日日期命名；该目录已有完整抓取时旧的整份自动挪到 `fetch-prev-MMDD-HHMM/`，不覆盖）

| 文件 | 用途 |
|---|---|
| `triage.md` | **先读这个**：论文以外的全部候选，按版面分组（实验室官方 / 模型 / 工具发布 / GitHub / 长文 / 播客 / 算力 / 政策 / 新闻 / 中文 / 社区），每组按偏好权重排序（`fetch/preferences.json`） |
| `triage-papers.md` | 论文：偏好权重 ≥3 带摘要，1–2 只列标题，其余按主题列标题。**全部扫一遍**，方向外的重大结果也要收 |
| `candidates.jsonl` | 一行一个候选（完整摘要、作者、主题、来源、`also_in`、`in_ledger`），需要细查时 `jq`/Python 读取 |
| `report.md` / `report.json` | 每个信源的条数、耗时、错误；`notes` 里有 arXiv 批次说明和警告 |

候选字段：`title url source channel kind published(本地 ISO) summary authors topics pref also_in in_ledger extra`。
- `pref` 只是排序用的偏好权重（见 `preferences.json`），**不是门槛**。
- `in_ledger`："pushed" = ledger.json 里已推过（★），"seen" = 看过未推（☆）；已推的只有出现实质新进展才再写，看过未推的可按新证据重新判断。
- `extra.discovery_only=true`（HF/GitHub trending、GitHub 搜索）：只是发现入口，热度不得作为纳入理由。
- `extra.date_unknown=true`：来自无 RSS 的网页差分（首次出现于本窗口），需打开原文核对日期。
- 热度数字（星标、点赞、下载、HN 分数）一律不写入输出。

## 信源

注册表：`sources.toml`（每条有 id / type / url / channel / kind / filter）。改信源只改这个文件。
文件末尾注释记录了验证过、已弃用的死链和停更源，以及替代方案。

| 频道 | 内容 |
|---|---|
| paper | arXiv 12 个分类 new+cross（cs.LG/CL/AI/CV/MA/SE/RO/CR/DC/HC/IR、stat.ML）、HF Daily Papers、Nature ML、Science |
| model | HF 约 70 个机构的新模型、HF trending 模型/数据集/Spaces（只看近 2–3 周新建的，过滤量化/合并/去审查等衍生物） |
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

X 由 agent 通过只读检索工具获取，见 `x-recall.zh-CN.md`（英文版 `x-recall.md`）。

## 时间与 arXiv 日历（自然日窗口）

- 窗口是**左闭右开** `[start,end)`，按每条的发布时间戳筛选（本地时间）。日报 = 前一个本地自然日；周报 = 上周一 00:00 → 本周一 00:00。
- **arXiv 列表日期 D 在 D 前一天 20:00 美东公开**（周日至周四晚）= 本地周一至周五 09:00（冬令时）/ 08:00（夏令时），所以一定落在 P 日窗口内；凌晨 00:30 跑时早已上线，**不等待、不补收**（`--catchup` 默认关）。本地周六、周日没有 arXiv 新批次。
- 工作日 P（公告时刻在窗口内的列表日）没有批次 = **真实召回故障**：`report.md` 的注里出现「⚠️ 工作日批次 … 在 /pastweek 与 /new 上都没有，已重试一次」，fetch 内部已重试一次（间隔 20 秒）。arXiv 偶有节假日不出批次（例：11/12 周四没有列表），所以 ⚠️ 要人工确认，不要自动当作「没有论文」。
- 节假日自动处理：脚本读 `pastweek` 页面的实际日期分段，不靠推算（pastweek 只保留最近 5 个公告日）。
- HF Daily：`daily_papers?date=D` 是 UTC 日期 D 策展的整榜（submittedOnDailyAt 全是 D T00:00Z，榜单在本地 D 08:00 → D+1 08:00 填满），周末没有榜单（0 篇是正常的）。日报（读写 state）取「运行时刻已完整收榜、且 `state['hf_daily:last']` 之后没取过」的全部 UTC 日期，归入本期（P 日凌晨运行 → 收榜到 UTC P-1），漏跑/周末空档自动补；周报、`--no-state`、自定义窗口：取收榜时刻落在窗口内的 UTC 日期（周报 7 个）。与 arXiv 在 dedupe 里按 arXiv id 合并（`also_in` 含 hf_daily），与 ledger 的去重照旧。

## 省 token 的小工具

- `fetch.py`：默认只打印失败源 + `注：` 行（`--verbose-sources` 恢复逐源输出）；论文「其余」档折叠（`--no-fold` 关闭，全量标题写 `triage-papers-rest.md`）；`--reuse` 复用同一窗口已抓过的结果（论文初筛先抓，日报复用）；同一天目录的完整抓取带文件锁。
- `page.py`：可一次传多个 URL（并行抓取）：`page.py URL1 "URL2@@关键词a,关键词b" URL3`。
- `xparse.py`：跳过转推和同文重复帖；结果带 `next_token` 时打印「⚠️ 截断」（补页规则见 `x-recall.zh-CN.md`）。
- `ledger.py add --log runs/P/filter-log.md`：同一份清单同时写台账并追加筛选日志的「推送」「看过未推」两节。
- `weekly_late.py` / `cand.py --since YYYY-MM-DD`：周报用，「后来才有信号、未推过」清单与多天目录一次查，只读已有抓取，不联网。
- `check_cands.py`：校验并改名论文初筛产物 `runs/P/paper-cands.md`。
- `troubleshoot-arxiv.zh-CN.md`（英文版 `troubleshoot-arxiv.md`）：arXiv 缺批次的处理（只在出现 ⚠️ 警告时读）。
- 提示词模板：`prompts/<en|zh-CN>/papers.md`（论文初筛，每天 00:10）、`daily.md`、`weekly.md`。
- 输出语言：`--lang en|zh-CN` 或环境变量 `DIGEST_LANG`（默认 `en`）决定 triage / report 里的标题与标签。

## 配置

- `DIGEST_TZ`：本地时区（IANA 名，默认 `UTC`），决定「自然日」窗口和显示时间。
- `fetch/preferences.json`（或 `DIGEST_PREFERENCES_FILE`）：主题偏好权重，格式见 `preferences.example.json`；只用于排序，不是门槛。
- `GITHUB_TOKEN` / `GH_TOKEN`（可选）：提高 GitHub API 限额。
