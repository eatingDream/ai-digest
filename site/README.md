# site/ — 加密静态站点 / encrypted static site

`publish.py` 把渲染好的日报/周报 HTML 变成一个**纯静态、客户端 AES-GCM 加密**的站点，可部署到 Cloudflare Pages
（无 Functions、无 Worker）。 / `publish.py` turns rendered issue HTML into a static site whose pages are
encrypted client-side (AES-256-GCM, key from PBKDF2-HMAC-SHA256, 600k iterations); deploy target: Cloudflare Pages.

## 配置 / Configuration

`site/CONFIG`（复制 `site/CONFIG.example`）或环境变量（环境变量优先）：

| 环境变量 | CONFIG 键 | 含义 |
|---|---|---|
| `SITE_DOMAIN` | `DOMAIN` | 站点域名 |
| `SITE_SLUG` | `SLUG` | 不可猜测的路径前缀（发布后不要改） |
| `CF_ACCOUNT_ID` | `ACCOUNT_ID` | Cloudflare 账号 ID |
| `CF_ZONE_ID` | `ZONE_ID` | （可选）域名所在 zone |
| `CF_PAGES_PROJECT` | `PROJECT` | Pages 项目名 |
| `DIGEST_SITE_PASSWORD_FILE` | — | 站点密码文件（默认 `~/.config/ai-digest/site-password`，权限 600） |
| `DIGEST_SITE_SALT_FILE` | — | PBKDF2 盐文件（默认 `~/.config/ai-digest/site-salt`，自动生成） |
| `CLOUDFLARE_API_TOKEN` / `_FILE` | — | 部署用的最小权限 token（Pages · Edit） |
| `DIGEST_LANG` | `LANG` | 站点输出语言 `en` / `zh-CN`（默认 `en`；命令行 `--lang` 优先） / Site UI language |
| `DIGEST_LANG_ALT` | `LANG_ALT` | （可选）另一语言伴生站点入口，如 `zh-CN:/zh-CN/` 或 `https://zh.digest.example.com/<slug>/` / Optional companion site link |
| `DIGEST_PRIVACY_PATTERNS_FILE` | — | 自定义发布前隐私扫描正则（默认 `site/privacy-patterns.txt`，已 gitignore） |

密码、盐、token **都不在仓库里**，也不会写入 `CONFIG` 或站点目录，更不会被打印。

## 页面加密 / Page encryption

- 每个页面（每期 + 目录页）发布时用 AES-256-GCM 加密；密钥 = PBKDF2-HMAC-SHA256(密码, 共享随机盐, 600,000 次)。
- 明文先 gzip 再加密（`"v":2,"z":"gzip"`），页面用 Web Crypto + `DecompressionStream` 解密。
- 所有站内链接都不带扩展名；`_headers` 设置 `noindex`、`nosniff`、`no-referrer` 与缓存策略；`robots.txt` 为 `Disallow: /`。
- 「记住此设备」默认勾选：派生密钥（不是密码）存 `localStorage`；不勾选则只存本标签页的 `sessionStorage`。「锁定」按钮清除两者。
- 这是**公共静态主机上的客户端加密**：密文可被下载。强密码和高迭代次数让离线猜测变贵，但不能替代「路径不公开」。
  阅读需要 JavaScript；首次解锁约需一两秒（600k 次迭代）。

## 发布 / Publish

```bash
python3 publish.py --issue daily/2026-01-15 --md digest-2026-01-15.md   # 增量：渲染 + 加密 + 部署 + 校验
python3 publish.py --issue weekly/2026-01-12_01-18 --md weekly-2026-01-12_01-18.md
python3 publish.py --prepare-only <html> daily 2026-01-15              # 只构建，不部署
python3 publish.py --rebuild | --rerender-all | --deploy-only | --rotate-password | --remove daily/LABEL
```

流程：保存干净源 → 注入 noindex 与「返回目录 / 上一期 / 下一期 / 本周周报」导航 → 重建目录页（写到 `site/plain/`）→
**对明文做隐私扫描**（邮箱 + 自定义规则，命中即失败）→ 加密写入 `site/public/<slug>/`（内容未变的页面复用旧密文，
见 `site/enc-cache.json`）→ `npx wrangler pages deploy` → HTTP 与解密校验。

## 演示站点（CI，无部署、无密钥）/ Demo build

```bash
DIGEST_DEMO_PASSWORD='demo' python3 publish.py --demo --out dist/                    # 英文 /en/ + 中文 /zh-CN/，页面间带语言切换
DIGEST_DEMO_PASSWORD='demo' python3 publish.py --demo --lang zh-CN --out dist/       # 只构建中文，输出在 dist/demo/
```

用 `examples/` 下的合成示例（`sample-digest.<lang>.md`、`sample-weekly.<lang>.md`，`<lang>` 为 `en` 或 `zh-CN`）构建一个加密演示站点到 `dist/`：
不需要 CONFIG / Cloudflare / 密码文件；密码仅来自环境变量 `DIGEST_DEMO_PASSWORD`（这是**公开的演示密码**）；
盐每次随机；构建后会逐页解密比对明文；`dist/_redirects` 把根路径跳到目录页。入口为 `/<SITE_SLUG 或 demo>/`。

## 轮换密码 / Rotate

`python3 publish.py --rotate-password`：生成新的 20 位密码与新盐（写入上面的文件，权限 600，不打印），
重新加密所有期并部署。旧密码和各浏览器里「记住此设备」的密钥随之失效。

校验：`node verify_decrypt.mjs --local` 或 `--url-base https://<SITE_DOMAIN>`（读密码文件，不打印密码）。

## 目录布局 / Layout

```
site/
  CONFIG(.example)         # PLATFORM, PROJECT, ACCOUNT_ID, ZONE_ID, DOMAIN, SLUG
  password_template.html   # 解锁页壳（无密钥）
  public/                  # 部署目录（robots.txt, _headers, 404.html 不加密；<slug>/ 下是密文）
  plain/                   # 导航注入后的明文（gitignore，永不部署）
  sources/                 # 干净 HTML（无导航），用来重建相邻页
```
