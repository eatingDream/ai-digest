# arXiv 缺批次时的处理（只在 report.md/fetch 输出的「注：」里出现「⚠️ 工作日批次」时才读本文件）

- **arXiv 缺批次的处理**：`P` 是工作日时批次早已上线，缺失是**真实召回故障**（也可能是 arXiv 当天放假，如 11/12 周四没有列表），不是「今天没有论文」。
  - 若 `report.md` 的注里有「⚠️ 工作日批次 … 在 /pastweek 与 /new 上都没有」（fetch 内部已重试过一次）：**读论文 triage 之前**再跑一次 `ai-digest fetch --daily --only arxiv,hf_daily`（约 20 秒；输出在 `runs/P/fetch-extra/`，**不会覆盖 `fetch/`**，论文读那里的 `triage-papers.md`——补查按窗口重取同一份 HF 整榜、不推进 `hf_daily:last`，所以 HF 论文也在里面）。仍没有就用 WebFetch 看一眼 `https://arxiv.org/list/cs.LG/new` 和 `https://arxiv.org/list/cs.LG/pastweek` 的标题日期，确认是 arXiv 放假/延迟还是抓取故障；确认是放假则论文版面按空处理，并在 `filter-log.md` 的「信源情况」写一行原因。**不要轮询、不要等待**；没有「13:00 补收」定时任务，也没有 catch-up。
  - 确认是真实故障后仍取不到：照常发布（论文版面空缺），在 `filter-log.md` 写明，**第二天**的运行不会自动补回这批（`--catchup` 默认关）；这是唯一允许写进推送消息的过程说明：在摘要后加一行「注：P 日（M/D）arXiv 批次未取到，论文版面暂缺」，之后不要停下来等待。
  - 一期日报的论文版面为空而 `P` 是工作日，几乎一定是召回失败。
