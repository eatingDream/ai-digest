# site/ — 静态站点与可选加密 / Static Site & Optional Encryption

`ai-digest publish` 把渲染好的日报/周报 HTML 构造成纵深加固的 Cloudflare Pages 静态站点：默认免密码根路径公开（`ENCRYPT=0`、`SLUG=`）并附带 Level 5/5 GEO 能力（含可选 `_worker.js`），可按需启用客户端 AES-256-GCM 加密。 / `ai-digest publish` builds a hardened Cloudflare Pages static site (public & root-mounted with Level 5/5 GEO by default, with opt-in client-side AES-256-GCM encryption).

## 配置 / Configuration

完整配置键与环境变量（环境变量优先于 `site/CONFIG`）见 [`site/CONFIG.example`](CONFIG.example) 及根目录 [README](../README.md#configuration) / [中文](../README.zh-CN.md#配置)。密码、盐与 token 均保存在仓库外（权限 600），绝不写入 `CONFIG` 或日志。

站点专用项 / Site-specific notes:
- `ENCRYPT` (`DIGEST_ENCRYPT`，默认 `0`)：设为 `1`/`true`/`yes`/`on` 开启客户端密码加密；`ai-digest publish --init --encrypt` 会同时生成 22 位随机 `SLUG` 并开启 `ENCRYPT=1`。
- `SLUG` (`SITE_SLUG`，默认留空)：留空即挂载在根路径 `/`（`/zh/`、`/en/`、`/latest`）；设置非空前缀（至少 8 位 URL 安全字符）可隐藏站点路径。
- `LANG_ALT` (`DIGEST_LANG_ALT`)：可选互链外部伴生站点（如 `zh-CN:/zh-CN/`）；当 `LANG=zh-CN` 且存在 `*.en.md` 时，`ai-digest publish` 会自动在同一站点下生成对称 `/zh/` 与 `/en/` 路径树。
- `ROOT_LATEST` (`DIGEST_ROOT_LATEST`，默认 `0`)：仅在设置了非空 `SLUG` 时生效；设为 `1` 才在根路径额外生成 `/latest`、`/daily/latest`、`/weekly/latest`（以及 `/zh/...`、`/en/...`）静态页（HTTP 200、URL 停留在 `latest`，加密模式下未解锁前不暴露 `SLUG`）。
- `site/privacy-patterns.txt` (`DIGEST_PRIVACY_PATTERNS_FILE`，已 gitignore)：发布前明文隐私扫描的额外自定义正则（覆盖中英文全部页面）。

## 路由与可选加密 / Routing & Optional Encryption

- **默认公开模式（`ENCRYPT=0`）**：通过隐私扫描后直接发布 `site/plain/` 到 `site/public/`，无密码输入页、无锁定按钮；自动生成完整 Level 5/5 GEO 产物（含 Content Signals 与 AI 爬虫规则的 `robots.txt`、`sitemap.xml`、`llms.txt`、`llms-full.txt`、`feed.xml`、JSON-LD、全站 `.md` 镜像、`.well-known/agent-skills/index.json`、`search-index.json` 与 Cloudflare Pages `_worker.js` 支持 `Accept: text/markdown` 与 `GET`/`HEAD`/`QUERY` `/.well-known/search`）；`ai-digest verify` 与 `node web/dist/verify_decrypt.mjs` 逐页校验 HTTP 200 及明文完全一致。
- **可选加密模式（`ENCRYPT=1`）**：每期与目录页先 gzip 再用 AES-256-GCM 加密（`"v":2,"z":"gzip"`），密钥来自 PBKDF2-HMAC-SHA256（共享随机盐，600,000 次迭代），浏览器端用内置编译的解锁页壳 + Web Crypto + `DecompressionStream` 解锁（`/en/` 路径自动显示英文解锁页；如本地存在 `<DIGEST_ROOT>/site/password_template.html` 仍兼容自定义模板）；自动清除所有公开 GEO 产物，保持 `robots.txt` 为 `Disallow: /` 与 `X-Robots-Tag: noindex, nofollow`。「记住此设备」仅存派生密钥（非密码）于 `localStorage`，「锁定」清除全部缓存。
- 站内链接无扩展名；`_headers` 保留 `nosniff`、`no-referrer`、SHA-256 CSP 与缓存策略（所有 `latest` 静态页路径均为 `no-store`，刷新即可加载最新一期且 URL 保持在 `latest`；公开模式仅对 `/404.html` 保留 `X-Robots-Tag: noindex, nofollow`）。

## 发布、演示与轮换 / Publish, Demo & Rotate

```bash
ai-digest publish --init [--encrypt]
ai-digest publish --issue daily/2026-01-15 --md digest-2026-01-15.md [--md-en digest-2026-01-15.en.md]
ai-digest publish --issue weekly/2026-01-12_01-18 --md weekly-2026-01-12_01-18.md
ai-digest publish --prepare-only <html> daily 2026-01-15
ai-digest publish --rebuild | --rerender-all | --deploy-only | --rotate-password | --remove daily/LABEL
ai-digest publish --demo --out dist/ [--lang zh-CN]   # 可选设 DIGEST_DEMO_PASSWORD='demo' 开启演示站加密
ai-digest verify --local                              # 或 --url-base https://<SITE_DOMAIN>（自动识别 ENCRYPT=0/1）
node web/dist/verify_decrypt.mjs --local              # 可选 Node WebCrypto 跨运行时对拍校验
```

- **增量发布流程**：保存干净源（`LANG=zh-CN` 自动发现同目录 `*.en.md` 并分别渲染至 `site/sources/` 与 `site/sources/en/`）→ 注入导航与语言切换按钮 → 重建 `/zh/`、`/en/` 及兼容根路径页面（`site/plain/`）→ 隐私扫描（命中即中止）→ 直接发布（或 `ENCRYPT=1` 时复用 `site/enc-cache.json` 增量加密）到 `site/public/` → 计算全站可执行内联脚本 SHA-256 CSP 并写入 `_headers` 与 `_worker.js` → Cloudflare Pages 部署 → HTTP 与逐页校验。
- **演示构建（`--demo`）**：用 `examples/` 合成数据构建站点到 `dist/`（默认 `/en/` + `/zh-CN/`，免密码公开；设置 `DIGEST_DEMO_PASSWORD` 时启用加密并逐页解密校验）。
- **轮换密码（`--rotate-password`，仅 `ENCRYPT=1`）**：生成新 20 位密码与新盐（写入模式 600 文件，不打印），全量重加密并部署，旧密码及各浏览器已记住密钥随之失效。

## 目录布局 / Layout

```text
site/
  CONFIG(.example)         # PLATFORM, PROJECT, ACCOUNT_ID, ZONE_ID, DOMAIN, SLUG, ENCRYPT, LANG, LANG_ALT, ROOT_LATEST
  public/                  # 部署目录（robots.txt, _headers, _redirects, 404.html 始终明文）
  plain/                   # 导航注入后的明文（gitignore，永不部署）
  sources/                 # 干净 HTML（无导航；含可选 en/ 子目录），用于重建相邻页
```
