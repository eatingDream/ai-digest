# site/ — 加密静态站点 / Encrypted Static Site

`publish.py` 把渲染好的日报/周报 HTML 构造成纯静态、客户端 AES-256-GCM 加密的 Cloudflare Pages 站点（无 Functions、无 Worker）。 / `publish.py` turns rendered HTML into a client-side AES-256-GCM encrypted static site for Cloudflare Pages.

## 配置 / Configuration

完整配置键与环境变量（环境变量优先于 `site/CONFIG`）见 [`site/CONFIG.example`](CONFIG.example) 及根目录 [README](../README.md#configuration) / [中文](../README.zh-CN.md#配置)。密码、盐与 token 均保存在仓库外（权限 600），绝不写入 `CONFIG` 或日志。

站点专用项 / Site-specific notes:
- `LANG_ALT` (`DIGEST_LANG_ALT`)：可选互链另一语言伴生站点（如 `zh-CN:/zh-CN/`） / optional companion language site link.
- `ROOT_LATEST` (`DIGEST_ROOT_LATEST`，默认 `0`)：设为 `1`/`true`/`yes`/`on` 才在根路径额外生成 `/latest`、`/daily/latest`、`/weekly/latest` 静态 302 跳转（默认关闭以防暴露 `SLUG`）；`/<slug>/{daily,weekly,}/latest` 始终自动写入 `_redirects`。
- `site/privacy-patterns.txt` (`DIGEST_PRIVACY_PATTERNS_FILE`，已 gitignore)：发布前明文隐私扫描的额外自定义正则（覆盖双语全部内容）。

## 页面加密与路由 / Encryption & Routing

- 每期与目录页发布时先 gzip 再用 AES-256-GCM 加密（`"v":2,"z":"gzip"`），密钥来自 PBKDF2-HMAC-SHA256（共享随机盐，600,000 次迭代），浏览器端用 Web Crypto + `DecompressionStream` 解锁（首次约 1–2 秒，需开启 JS）。
- 站内链接无扩展名；`robots.txt` 为 `Disallow: /`；`_headers` 设置 `noindex`、`nosniff`、`no-referrer` 与缓存策略（`latest` 跳转路径为 `no-store`）。
- 「记住此设备」默认勾选，仅存派生密钥（非密码）于 `localStorage`（不勾选存 `sessionStorage`）；「锁定」清除两者。客户端加密可提高离线破解成本，但仍依赖强密码与私密 `SLUG`。

## 发布、演示与轮换 / Publish, Demo & Rotate

```bash
python3 publish.py --issue daily/2026-01-15 --md digest-2026-01-15.md [--md-en digest-2026-01-15.en.md]
python3 publish.py --issue weekly/2026-01-12_01-18 --md weekly-2026-01-12_01-18.md
python3 publish.py --prepare-only <html> daily 2026-01-15
python3 publish.py --rebuild | --rerender-all | --deploy-only | --rotate-password | --remove daily/LABEL
DIGEST_DEMO_PASSWORD='demo' python3 publish.py --demo --out dist/ [--lang zh-CN]
node verify_decrypt.mjs --local   # 或 --url-base https://<SITE_DOMAIN>（读密码文件，不打印密码）
```

- **增量发布流程**：保存干净源 → 注入 `noindex` 与对应语言导航（`LANG=zh-CN` 自动合并同目录 `*.en.md`，`--rerender-all` 亦保留双语）→ 重建目录页（`site/plain/`）→ 隐私扫描（命中即中止）→ 加密输出到 `site/public/<slug>/`（未变更页面借 `site/enc-cache.json` 复用密文）→ `npx wrangler pages deploy` → HTTP 与解密校验。
- **演示构建（`--demo`）**：用 `examples/` 合成数据构建加密站点到 `dist/`（默认 `/en/` + `/zh-CN/`，带 `latest` 跳转与逐页解密校验）；无需 CONFIG 或密钥，每次随机盐，密码仅取自公开的 `DIGEST_DEMO_PASSWORD`。
- **轮换密码（`--rotate-password`）**：生成新 20 位密码与新盐（写入模式 600 文件，不打印），全量重加密并部署，旧密码及各浏览器已记住密钥随之失效。

## 目录布局 / Layout

```text
site/
  CONFIG(.example)         # PLATFORM, PROJECT, ACCOUNT_ID, ZONE_ID, DOMAIN, SLUG, LANG, LANG_ALT, ROOT_LATEST
  password_template.html   # 解锁页壳（无密钥）
  public/                  # 部署目录（robots.txt, _headers, _redirects, 404.html 不加密；<slug>/ 下是密文）
  plain/                   # 导航注入后的明文（gitignore，永不部署）
  sources/                 # 干净 HTML（无导航），用于重建相邻页
```
