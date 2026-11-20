# X 召回清单（定时任务 用，只读）

**调用协议（省 token，2026-11-17 起）**
1. 每次 `search_posts_all` / `search_news` 用 agent 的 X 只读检索工具并让结果落盘（`write_to_file`），**结果落盘、不进上下文**（原来每次调用把 6–20KB 的 JSON 倒进上下文，7–9 次约 100KB+）。参数：`max_results` 50（A/B/C 组；之前默认 25 在 A2、B1、C1 都被截断，召回有缺口）、`start_time`/`end_time` = 窗口、`post.fields=["created_at"]`、`expansions=["author_id"]`、`user.fields=["username"]`；B、C 组查询末尾加 `-is:reply`。
2. 全部跑完后**只跑一次**解析：`python3 fetch/xparse.py A1=<文件> A2=<文件> A3=<文件> B1=… B2=… C1=… C2=… E1=… --out runs/D/x-raw.md`。它跨组去重、去互动数字，打印第一行条数（`A1=20 A2=25 …`，原样抄进 `x-leads.md` 开头）和每条一行线索（约 200 字节/条）。这一份输出就是 X 线索，读一遍即可，**不要再 cat 原始 JSON，不要自己写解析脚本**。
2b. **截断补页（2026-11-18 起）**：连接器每次最多回 25 条，`max_results` 50 不一定生效。xparse 第二行出现「⚠️ 截断」时：注意全量检索即使只回几条也可能带 `next_token`（例：11/17 夜间 A1 只回 2 条、最早 12:13，窗口前 3 小时没取到）。A、B 组和 C 组账号那条（C1）用它打印的 `next_token` 补页（参数不变，加 `next_token`；同一个 block 并行、照样 `write_to_file`），然后把补页文件加进**同一条** xparse 命令重跑（标签如 `B1p2=…`）：A 组（官方账号，最重要）补到没有 `next_token` 为止、最多再 2 页；B、C1 只补 1 页。C 组关键词那条（C2）和 D 组主题检索量太大（25 条只覆盖几分钟），不翻页，改为一开始就传 `sort_order: "relevancy"`。
3. 预算：A×3 + B×2 + C×2 = 7 次必跑；E（`search_news`）按「行业与产品」「政策」版面薄厚跑 2–3 个关键词；D 组默认不跑，仅当 triage + X 线索里某个版面（如具身、融资、人事）明显空白、或某条线索需要交叉验证时，挑 1–2 个相关主题跑。一次检索没有新信息就不要换写法重试。
4. 只追值得写的线索：把 X 线索和 `triage.md` 对一遍（同一事件通常两边都有），X 独有的才需要追一手出处；写进 `runs/D/x-leads.md` 的只记链接+一句话，≤ 25 行。

X 不走脚本，由 agent 用只读的 X 检索工具（连接器 / API）检索。**只读**：不发帖、不点赞、不关注、不收藏、不发私信。
工具：`search_posts_all`（务必传 `start_time` / `end_time` = 窗口，UTC ISO8601）、`search_news` / `get_news`（X 新闻聚合）、`get_users_posts`。
每条查询都加 `-is:retweet`；字段见上方「调用协议」（`post.fields=created_at`、`expansions=author_id`、`user.fields=username`）。
`min_likes:` 之类只能作发现时的去噪手段，不能作为纳入理由，也不写进正文。

## A. 官方与实验室账号（每次必跑；按 1024 字符上限拆成 3 组）

1. `(from:OpenAI OR from:OpenAIDevs OR from:AnthropicAI OR from:claudeai OR from:GoogleDeepMind OR from:GoogleAI OR from:googlegemma OR from:GeminiApp OR from:AIatMeta OR from:MistralAI OR from:xai OR from:MSFTResearch OR from:NVIDIAAI OR from:nvidia OR from:huggingface OR from:thinkymachines OR from:cohere OR from:allen_ai OR from:togethercompute) -is:retweet -is:reply`
2. `(from:deepseek_ai OR from:Alibaba_Qwen OR from:QwenDevs OR from:Kimi_Moonshot OR from:Zai_org OR from:MiniMax_AI OR from:StepFun_ai OR from:TencentHunyuan OR from:BytedanceTalk OR from:OpenBMB OR from:intern_lm OR from:Xiaomi_MiMo OR from:SakanaAILabs OR from:perplexity_ai OR from:cursor_ai OR from:cognition OR from:windsurf OR from:Replit OR from:LangChainAI OR from:vllm_project OR from:lmsysorg OR from:llama_cpp OR from:ollama) -is:retweet -is:reply`
3. `(from:METR_Evals OR from:EpochAIResearch OR from:ArtificialAnlys OR from:lmarena_ai OR from:AISecurityInst OR from:apolloaievals OR from:GoodfireAI OR from:TransluceAI OR from:PrimeIntellect OR from:NousResearch OR from:physical_int OR from:Figure_robot OR from:UnitreeRobotics OR from:SemiAnalysis_ OR from:StanfordHAI OR from:CSETGeorgetown) -is:retweet -is:reply`

## B. 研究者 / 工程师 / 评论者（每次必跑，2 组）

1. `(from:karpathy OR from:natolambert OR from:_jasonwei OR from:polynoamial OR from:soumithchintala OR from:rasbt OR from:omarsar0 OR from:dair_ai OR from:TheTuringPost OR from:_akhaliq OR from:arankomatsuzaki OR from:iScienceLuvr OR from:teortaxesTex OR from:swyx OR from:simonw OR from:emollick OR from:Miles_Brundage OR from:jackclarkSF OR from:ylecun OR from:demishassabis OR from:sama OR from:JeffDean OR from:ilyasut OR from:drfeifei) -is:retweet`
2. `(from:<account_1> OR from:<account_2> OR … ) -is:retweet`（读者自己的研究者/工程师圈子：把你信任的账号填在这里，每组 ≤ 1024 字符；见 `x-sources.md`）

## C. 中文圈（每次必跑）

`(from:<zh_account_1> OR from:<zh_account_2> OR … ) -is:retweet`（读者自己的中文圈子账号，示例见 `x-sources.md`）
以及关键词：`(发布 OR 开源 OR 上线 OR 论文 OR 融资 OR 收购) (模型 OR 大模型 OR 智能体 OR Agent OR AI) lang:zh -is:retweet`（只取窗口内，仅作线索，一手出处再核）

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
