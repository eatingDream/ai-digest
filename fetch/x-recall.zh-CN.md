# X 召回清单（可选、按工具能力启用，只读）

**能力与占位符门控**
- **按运行器能力可选执行**：如果当前 agent 运行环境没有提供只读 X 检索工具（`search_posts_all`、`get_users_posts`、`search_news`、`get_news` 等，例如未接入 X 连接器/MCP 的标准 Claude Code、Cursor、Codex），**直接干净跳过下方连接器协议，不要报错中断**——可以直接仅靠 `fetch/fetch.py` 抓取的约 155 个公开信源成稿，或选用下方的「无 Grok Bot 安全替代方案」。
- **自动跳过未替换的账号占位符**：下文 **B2** 和 **C1** 含有 `<account_1>` / `<zh_account_1>` 个人关注圈模板占位符。调用前先检查是否已替换成真实账号名——若仍含 `<account_1>` 或 `<zh_account_1>`，**直接跳过该条查询**（记为 `B2=未配置跳过` / `C1=未配置跳过`），切勿把字面量 `<...>` 发给检索接口。

## 没有 Grok Bot 时如何安全抓取 X 信源（免费与付费途径）

不依赖 Grok Bot、也不用交出个人 X 账号 Cookie，同样可以安全获取 X 上的 AI 动态。按预算和需求选择即可：

| 方案 | 费用 | 封号 / 账号风险 | 接入 AI Digest 方式 |
|---|---|---|---|
| **1. 纯公开信源模式（内置默认）** | **免费** | **零风险** | 无需额外配置：`fetch/fetch.py` 默认抓取约 155 个 RSS/Atom/arXiv/HF/GitHub/官方博客/HN/Reddit/YouTube 信源，绝大多数一手发布已覆盖。 |
| **2. 只读网页搜索引擎检索**（`site:x.com/<账号>`） | **免费额度**（Gemini 搜索接地、Brave Search、Tavily、Exa、Perplexity 等） | **零风险**（不登录 X） | 让 agent 用自带网页搜索工具按日期窗口检索 A 组核心实验室账号的公开动态，将核实后的线索写入 `runs/<P>/x-leads.md`，再一律追到官方博客/论文/仓库一手链接。 |
| **3. 跨平台官方 RSS 与自建桥接** | **免费** | **零风险** | 许多研究者会在 Bluesky（原生 RSS：`https://bsky.app/profile/<handle>/rss`）、Mastodon、Substack 或官网博客同步发帖，可直接加进 `fetch/sources.toml`；若自建 [RSSHub](https://docs.rsshub.app/)，仅使用公开只读路由并保持低频轮询。 |
| **4. xAI Grok 官方 API（`x_search` 工具）** *（付费首选）* | **付费**（按量计费，每日运行成本极低） | **零风险**（官方 API，无需 X 账号/Cookie） | 调用 xAI Responses API（`https://api.x.ai/v1/responses`）并启用内置 `x_search` 工具（支持 `allowed_x_handles` 与 `from_date`/`to_date`），套用下文 A–C 组账号清单即可，结果整理成「链接 + 一句话」写入 `runs/<P>/x-leads.md`。密钥 `XAI_API_KEY` 放在环境变量或仓库外 `chmod 600` 文件中。 |
| **5. X 官方 API v2（Pay-Per-Use / Basic 档）** | **付费** | **零风险**（官方只读 Bearer Token） | 用只读 Bearer Token 调用 `GET /2/tweets/search/recent`，直接复用下文 A–D 组查询语法（传 `start_time`、`end_time`、`tweet.fields=created_at`、`expansions=author_id`、`user.fields=username`），JSON 落盘后直接喂给 `python3 fetch/xparse.py` 去重解析。*（注：X 官方 **Free** 免费档不包含时间线与搜索读取接口，请勿误购。）* |
| **6. 第三方只读 X / 搜索 API 与 MCP** | **付费**（按调用量） | **低风险**（无需个人 X 账号） | 通过 Exa、Tavily、Perplexity Sonar、TwitterAPI.io 或 Apify 等只读接口/MCP 检索公开推文，整理到 `runs/<P>/x-leads.md`，每条仍需追到一手出处。 |

**安全红线（严禁触碰）：**
- **严禁把个人主号 Cookie（`auth_token` / `ct0`）塞进无头浏览器或非官方爬虫**：封号与会话泄露风险极高。
- **严格只读**：自动跑批的 agent 不发帖、不点赞、不收藏、不关注、不发私信。
- **密钥与关注圈彻底隔离**：`XAI_API_KEY`、`X_BEARER_TOKEN` 等凭据只存环境变量或仓库外 `0600` 权限文件，严禁写进 `site/CONFIG` 或日志；个人关注列表也绝不泄露到公开日报正文。

**调用协议（省 token，有 X 检索工具时启用）**
1. 每次 `search_posts_all` / `search_news` 用 agent 的 X 只读检索工具并让结果落盘（`write_to_file`，若连接器不支持该参数则由 Shell/文件工具落盘），**结果落盘、不留在上下文**（原来每次调用把 6–20KB 的 JSON 倒进上下文，7–9 次约 100KB+）。参数：`max_results` 50（A/B/C 组；之前默认 25 在 A2、B1、C1 都被截断，召回有缺口）、`start_time`/`end_time` = 窗口、`post.fields=["created_at"]`、`expansions=["author_id"]`、`user.fields=["username"]`；B、C 组查询末尾加 `-is:reply`。
2. 全部跑完后**只跑一次**解析：`python3 fetch/xparse.py A1=<文件> A2=<文件> A3=<文件> B1=… C2=… [B2=… C1=…] E1=… --out runs/D/x-raw.md`（`B2` / `C1` 仅在已配置真实账号时传入）。它跨组去重、去互动数字，打印第一行条数（`A1=20 A2=25 …`，原样抄进 `x-leads.md` 开头）和每条一行线索（约 200 字节/条）。这一份输出就是 X 线索，读一遍即可，**不要再 cat 原始 JSON，不要自己写解析脚本**。
2b. **截断补页（2026-11-18 起）**：连接器每次最多回 25 条，`max_results` 50 不一定生效。xparse 第二行出现「⚠️ 截断」时：注意全量检索即使只回几条也可能带 `next_token`（例：11/17 夜间 A1 只回 2 条、最早 12:13，窗口前 3 小时没取到）。A、B 组和 C 组账号那条（C1，若已启用）用它打印的 `next_token` 补页（参数不变，加 `next_token`；同一个 block 并行、照样落盘），然后把补页文件加进**同一条** xparse 命令重跑（标签如 `B1p2=…`）：A 组（官方账号，最重要）补到没有 `next_token` 为止、最多再 2 页；B、C1 只补 1 页。C 组关键词那条（C2）和 D 组主题检索量太大（25 条只覆盖几分钟），不翻页，改为一开始就传 `sort_order: "relevancy"`。
3. 预算：A×3 + B1 + C2 = 5 次核心调用（替换 `<account_1>` / `<zh_account_1>` 后再加 B2、C1，共 7 次）；E（`search_news`，若工具支持）按「行业与产品」「政策」版面薄厚跑 2–3 个关键词；D 组默认不跑，仅当 triage + X 线索里某个版面（如具身、融资、人事）明显空白、或某条线索需要交叉验证时，挑 1–2 个相关主题跑。一次检索没有新信息就不要换写法重试。
4. 只追值得写的线索：把 X 线索和 `triage.md` 对一遍（同一事件通常两边都有），X 独有的才需要追一手出处；写进 `runs/D/x-leads.md` 的只记链接+一句话，≤ 25 行。

X 不走脚本，由 agent 用只读的 X 检索工具（连接器 / API）检索。**只读**：不发帖、不点赞、不关注、不收藏、不发私信。
工具：`search_posts_all`（务必传 `start_time` / `end_time` = 窗口，UTC ISO8601）、`search_news` / `get_news`（X 新闻聚合）、`get_users_posts`。
每条查询都加 `-is:retweet`；字段见上方「调用协议」（`post.fields=created_at`、`expansions=author_id`、`user.fields=username`）。
`min_likes:` 之类只能作发现时的去噪手段，不能作为纳入理由，也不写进正文。

## A. 官方与实验室账号（有 X 检索工具时必跑；按 1024 字符上限拆成 3 组）

1. `(from:OpenAI OR from:OpenAIDevs OR from:AnthropicAI OR from:claudeai OR from:GoogleDeepMind OR from:GoogleAI OR from:googlegemma OR from:GeminiApp OR from:AIatMeta OR from:MistralAI OR from:xai OR from:MSFTResearch OR from:NVIDIAAI OR from:nvidia OR from:huggingface OR from:thinkymachines OR from:cohere OR from:allen_ai OR from:togethercompute) -is:retweet -is:reply`
2. `(from:deepseek_ai OR from:Alibaba_Qwen OR from:QwenDevs OR from:Kimi_Moonshot OR from:Zai_org OR from:MiniMax_AI OR from:StepFun_ai OR from:TencentHunyuan OR from:BytedanceTalk OR from:OpenBMB OR from:intern_lm OR from:Xiaomi_MiMo OR from:SakanaAILabs OR from:perplexity_ai OR from:cursor_ai OR from:cognition OR from:windsurf OR from:Replit OR from:LangChainAI OR from:vllm_project OR from:lmsysorg OR from:llama_cpp OR from:ollama) -is:retweet -is:reply`
3. `(from:METR_Evals OR from:EpochAIResearch OR from:ArtificialAnlys OR from:lmarena_ai OR from:AISecurityInst OR from:apolloaievals OR from:GoodfireAI OR from:TransluceAI OR from:PrimeIntellect OR from:NousResearch OR from:physical_int OR from:Figure_robot OR from:UnitreeRobotics OR from:SemiAnalysis_ OR from:StanfordHAI OR from:CSETGeorgetown) -is:retweet -is:reply`

## B. 研究者 / 工程师 / 评论者（有 X 工具时 B1 必跑；B2 配置后启用）

1. `(from:karpathy OR from:natolambert OR from:_jasonwei OR from:polynoamial OR from:soumithchintala OR from:rasbt OR from:omarsar0 OR from:dair_ai OR from:TheTuringPost OR from:_akhaliq OR from:arankomatsuzaki OR from:iScienceLuvr OR from:teortaxesTex OR from:swyx OR from:simonw OR from:emollick OR from:Miles_Brundage OR from:jackclarkSF OR from:ylecun OR from:demishassabis OR from:sama OR from:JeffDean OR from:ilyasut OR from:drfeifei) -is:retweet`
2. *（可选——若 `<account_1>` 未替换则直接跳过）* `(from:<account_1> OR from:<account_2> OR … ) -is:retweet`（读者自己的研究者/工程师圈子：把你信任的账号填在这里，每组 ≤ 1024 字符；见 `x-sources.zh-CN.md`）

## C. 中文圈（有 X 工具时 C2 关键词必跑；C1 账号组配置后启用）

*（C1，可选——若 `<zh_account_1>` 未替换则直接跳过）* `(from:<zh_account_1> OR from:<zh_account_2> OR … ) -is:retweet`（读者自己的中文圈子账号，示例见 `x-sources.zh-CN.md`）
以及关键词 *（C2）*：`(发布 OR 开源 OR 上线 OR 论文 OR 融资 OR 收购) (模型 OR 大模型 OR 智能体 OR Agent OR AI) lang:zh -is:retweet`（只取窗口内，仅作线索，一手出处再核）

## D. 主题检索（广域，按需全部跑；每组只作线索，必须追到一手出处）

| 类型 | 查询 |
|---|---|
| 模型发布 | `("open weights" OR "open-weight" OR "now available" OR "introducing" OR "we're releasing") (model OR LLM) -is:retweet lang:en` |
| 论文 | `(arxiv.org OR huggingface.co/papers) (RL OR agent OR "post-training" OR distillation OR reasoning OR harness OR "self-improving") -is:retweet` |
| Harness / agent 工程 | `("agent harness" OR "coding agent" OR "Claude Code" OR Codex OR "computer use" OR MCP OR "agent skills") -is:retweet lang:en` |
| 算力 / 芯片 | `(GPU OR TPU OR Blackwell OR Rubin OR HBM OR "data center" OR datacenter OR "export controls") (AI OR NVIDIA OR AMD OR TSMC OR Huawei) -is:retweet` |
| 融资 / 并购 | `("raises" OR "Series A" OR "Series B" OR "Series C" OR valuation OR acquires OR acquisition) (AI OR LLM OR agent) -is:retweet lang:en` |
| 政策 / 安全 | `("AI Act" OR "executive order" OR "AI safety" OR "AI Security Institute" OR regulation OR lawsuit) AI -is:retweet lang:en` |
| 人事 | `("joins" OR "joined" OR "leaving" OR "departs" OR "steps down" OR "new CEO") (OpenAI OR Anthropic OR DeepMind OR xAI OR Meta OR NVIDIA OR "Thinking Machines" OR SSI) -is:retweet lang:en` |
| 长文 / 访谈 | `(podcast OR interview OR essay OR "new post" OR talk OR lecture) (AI OR LLM OR AGI) has:links -is:retweet lang:en` |
| 具身 / 机器人 | `(humanoid OR robotics OR "robot foundation model" OR VLA OR embodied) -is:retweet lang:en` |

## E. X 新闻聚合

`search_news`：`AI model`、`OpenAI`、`Anthropic`、`Google Gemini`、`NVIDIA`、`AI regulation`、`AI funding`、`DeepSeek`、`Qwen`；`get_news` 取当天科技类。用来补「行业与产品」「政策」两个版面。

## 使用原则

- X 帖子只是线索：模型 → HF / 官方博客；论文 → arXiv；融资/人事 → 公司公告或可靠媒体；政策 → 官方文件。追不到一手出处的，标注「仅见 X」或不收。
- 读者的圈子（B2、C 组，示例账号清单见 `x-sources.md`）只作偏好参考和平局加权，不是边界；圈子没提但实质重要的照样收。
- 把检索到的线索写进 `runs/<日期>/x-leads.md`（只记链接与一句话，不记作者个人信息之外的东西），便于周报复查。
