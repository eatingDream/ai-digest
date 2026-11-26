export interface WorkerConfig {
  origin: string;
  contentSignal: string;
  linkHeader: string;
  searchEnabled: boolean;
  globalHeaders: Record<string, string>;
  cacheRules: Array<[string, string]>;
  redirects: Array<[string, string, number]>;
}

interface SearchDoc {
  t?: string;
  d?: string;
  l?: string;
  s?: string;
  g?: string[];
  u?: string;
  m?: string;
  p?: string;
  lk?: string[];
}

interface SearchIndex {
  docs: SearchDoc[];
}

interface WorkerEnv {
  ASSETS: {
    fetch(request: Request): Promise<Response>;
  };
}

declare const __WORKER_CFG_JSON__: WorkerConfig;
const CFG: WorkerConfig = __WORKER_CFG_JSON__;

const cachedIndexes = new Map<string, SearchIndex>();

function prefersMarkdown(accept: string | null): boolean {
  if (!accept) return false;
  let mdQ = -1;
  let htmlQ = -1;
  let anyQ = -1;
  for (const part of accept.split(",")) {
    const segs = part.trim().split(";");
    const mime = (segs[0] || "").trim().toLowerCase();
    if (!mime) continue;
    let q = 1.0;
    for (let i = 1; i < segs.length; i++) {
      const kv = segs[i].trim().split("=");
      if (kv.length === 2 && kv[0].trim().toLowerCase() === "q") {
        const n = Number.parseFloat(kv[1]);
        if (Number.isFinite(n)) q = Math.max(0, Math.min(1, n));
      }
    }
    if (mime === "text/markdown" || mime === "text/x-markdown") {
      mdQ = Math.max(mdQ, q);
    } else if (mime === "text/html" || mime === "application/xhtml+xml") {
      htmlQ = Math.max(htmlQ, q);
    } else if (mime === "*/*" || mime === "text/*") {
      anyQ = Math.max(anyQ, q);
    }
  }
  return mdQ > 0 && mdQ > htmlQ && mdQ >= anyQ;
}

function matchCacheRule(pathname: string): string | null {
  let matched: string | null = null;
  for (const [pat, val] of CFG.cacheRules) {
    if (pat === pathname) {
      matched = val;
    } else if (pat.endsWith("/*")) {
      const prefix = pat.slice(0, -1);
      if (pathname.startsWith(prefix)) matched = val;
    }
  }
  return matched;
}

function applyBaseHeaders(
  headers: Headers,
  pathname: string,
  status: number,
  extra?: Record<string, string>
): void {
  for (const [k, v] of Object.entries(CFG.globalHeaders)) {
    headers.set(k, v);
  }
  headers.set("Content-Signal", CFG.contentSignal);
  headers.set("Link", CFG.linkHeader);
  headers.set("Vary", "Accept");
  if (status === 404) {
    headers.set("X-Robots-Tag", "noindex, nofollow");
    headers.set("Cache-Control", "no-store");
  } else {
    const cc = matchCacheRule(pathname);
    if (cc) headers.set("Cache-Control", cc);
  }
  if (extra) {
    for (const [k, v] of Object.entries(extra)) {
      headers.set(k, v);
    }
  }
}

function mapToMdCandidates(pathname: string): string[] {
  const clean = pathname.replace(/\/+/g, "/");
  if (clean.endsWith("/index.md")) {
    const base = clean.slice(0, -"/index.md".length);
    return base ? [clean, base + ".md"] : ["/index.md"];
  }
  if (clean.endsWith(".md")) return [clean];
  if (clean === "/" || clean === "/index" || clean === "/index.html") {
    return ["/index.md"];
  }
  if (clean.endsWith("/")) {
    return [clean + "index.md", clean.slice(0, -1) + ".md"];
  }
  if (clean.endsWith(".html")) return [clean.slice(0, -5) + ".md"];
  return [clean + ".md", clean + "/index.md"];
}

function tokenize(text: string): Set<string> {
  const out = new Set<string>();
  if (!text) return out;
  const low = String(text).toLowerCase();
  const ascii = low.match(/[a-z0-9][a-z0-9._+-]*/g) || [];
  for (const w of ascii) {
    if (w.length >= 2 || /^[0-9]$/.test(w)) out.add(w);
  }
  const cjkRuns = low.match(/[\u4e00-\u9fff]+/g) || [];
  for (const run of cjkRuns) {
    for (let i = 0; i < run.length; i++) {
      out.add(run[i]);
      if (i + 1 < run.length) out.add(run.slice(i, i + 2));
    }
  }
  return out;
}

async function loadSearchIndex(
  env: WorkerEnv,
  origin: string
): Promise<SearchIndex | null> {
  const cached = cachedIndexes.get(origin);
  if (cached) return cached;
  const resp = await env.ASSETS.fetch(new Request(origin + "/search-index.json"));
  if (!resp.ok) return null;
  const parsed = (await resp.json()) as SearchIndex;
  cachedIndexes.set(origin, parsed);
  return parsed;
}

async function readBodyWithLimit(
  request: Request,
  maxBytes: number
): Promise<string | null> {
  const cl = request.headers.get("content-length");
  if (cl !== null) {
    const parsedLen = Number.parseInt(cl, 10);
    if (Number.isFinite(parsedLen) && parsedLen > maxBytes) {
      await request.body?.cancel("body too large").catch(() => {});
      return null;
    }
  }
  if (!request.body) return "";
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value || value.byteLength === 0) continue;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel("body too large").catch(() => {});
        return null;
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const merged = new Uint8Array(total);
  let offset = 0;
  for (const c of chunks) {
    merged.set(c, offset);
    offset += c.byteLength;
  }
  return new TextDecoder("utf-8").decode(merged);
}

async function handleSearch(
  request: Request,
  env: WorkerEnv,
  url: URL
): Promise<Response> {
  const method = request.method.toUpperCase();
  if (method !== "GET" && method !== "HEAD" && method !== "QUERY") {
    return new Response("Method Not Allowed\n", {
      status: 405,
      headers: {
        "Content-Type": "text/plain; charset=utf-8",
        Allow: "GET, HEAD, QUERY",
        "Accept-Query": "text/plain, application/json",
      },
    });
  }

  let q = (url.searchParams.get("q") || "").trim();
  let langFilter = (url.searchParams.get("lang") || "").trim().toLowerCase();
  let limit = Number.parseInt(url.searchParams.get("limit") || "10", 10);

  if (method === "QUERY") {
    const raw = await readBodyWithLimit(request, 4096);
    if (raw === null) {
      return new Response("Query body too large (max 4096 bytes).\n", {
        status: 413,
        headers: {
          "Content-Type": "text/markdown; charset=utf-8",
          "Accept-Query": "text/plain, application/json",
        },
      });
    }
    const ct = (request.headers.get("content-type") || "").toLowerCase();
    if (ct.includes("application/json") || raw.trim().startsWith("{")) {
      try {
        const body = JSON.parse(raw) as {
          q?: unknown;
          lang?: unknown;
          limit?: unknown;
        };
        if (typeof body.q === "string") q = body.q.trim();
        if (typeof body.lang === "string") {
          langFilter = body.lang.trim().toLowerCase();
        }
        if (typeof body.limit === "number" && Number.isFinite(body.limit)) {
          limit = body.limit;
        }
      } catch {
        return new Response('Invalid JSON body; expected {"q": "..."}.\n', {
          status: 400,
          headers: {
            "Content-Type": "text/markdown; charset=utf-8",
            "Accept-Query": "text/plain, application/json",
          },
        });
      }
    } else {
      q = raw.trim();
    }
  }

  q = q.replace(/[\x00-\x1f\x7f]/g, " ").trim().slice(0, 300);
  if (!Number.isFinite(limit) || limit < 1) limit = 10;
  if (limit > 20) limit = 20;

  const baseMdHeaders: Record<string, string> = {
    "Content-Type": "text/markdown; charset=utf-8",
    "Accept-Query": "text/plain, application/json",
    "Content-Signal": CFG.contentSignal,
    "X-Content-Type-Options": "nosniff",
    "Cache-Control": "public, max-age=300",
  };

  if (!q) {
    const usage =
      "# Search for Agents\n\n" +
      "Provide a query via `GET /.well-known/search?q=<query>` or `QUERY /.well-known/search` " +
      'with `text/plain` or JSON `{"q": "..."}`.\n';
    return new Response(method === "HEAD" ? null : usage, {
      status: 400,
      headers: baseMdHeaders,
    });
  }

  const idx = await loadSearchIndex(env, url.origin);
  if (!idx || !Array.isArray(idx.docs)) {
    return new Response(method === "HEAD" ? null : "Search index unavailable.\n", {
      status: 503,
      headers: { ...baseMdHeaders, "Retry-After": "60" },
    });
  }

  const qLow = q.toLowerCase();
  const qToks = [...tokenize(q)];
  const scored: Array<{ score: number; doc: SearchDoc }> = [];

  for (const doc of idx.docs) {
    if (langFilter) {
      const dl = (doc.l || "").toLowerCase();
      if (dl !== langFilter && !dl.startsWith(langFilter)) continue;
    }
    const titleLow = (doc.t || "").toLowerCase();
    const tagsLow = (doc.g || []).join(" ").toLowerCase();
    const srcLow = (doc.s || "").toLowerCase();
    const snipLow = (doc.p || "").toLowerCase();

    let score = 0;
    if (titleLow.includes(qLow)) score += 10;
    if (snipLow.includes(qLow)) score += 5;
    if (tagsLow.includes(qLow)) score += 6;
    for (const tok of qToks) {
      const w = tok.length >= 2 ? 1.5 : 0.4;
      if (titleLow.includes(tok)) score += 4.0 * w;
      if (tagsLow.includes(tok)) score += 3.0 * w;
      if (srcLow.includes(tok)) score += 2.0 * w;
      if (snipLow.includes(tok)) score += 1.0 * w;
    }
    if (score > 0) {
      scored.push({ score, doc });
    }
  }

  scored.sort(
    (a, b) => b.score - a.score || (b.doc.d || "").localeCompare(a.doc.d || "")
  );
  const top = scored.slice(0, limit);
  const maxScore = top.length ? top[0].score : 1;

  const escQ = q.replace(/"/g, "'");
  const lines: string[] = [`# Search results for: "${escQ}"`, ""];
  if (!top.length) {
    lines.push(
      "No matching items found. Try broader keywords or check `/llms.txt` for all briefings."
    );
  } else {
    for (const { score, doc } of top) {
      const norm = (score / maxScore).toFixed(3);
      const relMd = (doc.m || "").replace(/^\//, "");
      const tags = (doc.g || []).join(", ");
      lines.push(`## ${relMd} (score: ${norm})`);
      lines.push("");
      lines.push(`### ${doc.t || ""}`);
      const meta = [`Date: ${doc.d || ""}`, `Lang: ${doc.l || ""}`];
      if (doc.s) meta.push(`Source: ${doc.s}`);
      if (tags) meta.push(`Tags: ${tags}`);
      lines.push(`- ${meta.join(" | ")}`);
      lines.push(`- URL: ${doc.u || ""}`);
      lines.push(`- Markdown: ${CFG.origin}${doc.m || ""}`);
      if (doc.p) {
        lines.push("");
        lines.push(doc.p);
      }
      if (Array.isArray(doc.lk) && doc.lk.length) {
        lines.push("");
        lines.push("Sources: " + doc.lk.join(" , "));
      }
      lines.push("");
      lines.push("---");
      lines.push("");
    }
  }

  const body = lines.join("\n").trimEnd() + "\n";
  const byteLen = new TextEncoder().encode(body).byteLength;
  const headers = new Headers(baseMdHeaders);
  headers.set("x-markdown-tokens", String(Math.max(1, Math.floor(byteLen / 4))));
  return new Response(method === "HEAD" ? null : body, { status: 200, headers });
}

export default {
  async fetch(request: Request, env: WorkerEnv): Promise<Response> {
    const url = new URL(request.url);
    const pathname = url.pathname;

    if (CFG.searchEnabled && pathname === "/.well-known/search") {
      return handleSearch(request, env, url);
    }

    for (const [src, dst, code] of CFG.redirects) {
      if (pathname === src) {
        return Response.redirect(new URL(dst, url.origin).toString(), code);
      }
    }

    if (pathname.endsWith(".md")) {
      for (const cand of mapToMdCandidates(pathname)) {
        const mdResp = await env.ASSETS.fetch(
          new Request(new URL(cand, url.origin), {
            method: request.method === "HEAD" ? "HEAD" : "GET",
          })
        );
        if (mdResp.ok) {
          const buf =
            request.method === "HEAD" ? null : await mdResp.arrayBuffer();
          const headers = new Headers(mdResp.headers);
          const tok = buf ? Math.max(1, Math.floor(buf.byteLength / 4)) : 1;
          applyBaseHeaders(headers, pathname, 200, {
            "Content-Type": "text/markdown; charset=utf-8",
            "x-markdown-tokens": String(tok),
          });
          return new Response(buf, { status: 200, headers });
        }
      }
    }

    if (
      (request.method === "GET" || request.method === "HEAD") &&
      prefersMarkdown(request.headers.get("accept"))
    ) {
      for (const cand of mapToMdCandidates(pathname)) {
        const mdResp = await env.ASSETS.fetch(
          new Request(new URL(cand, url.origin), {
            method: request.method === "HEAD" ? "HEAD" : "GET",
          })
        );
        if (mdResp.ok) {
          const buf =
            request.method === "HEAD" ? null : await mdResp.arrayBuffer();
          const headers = new Headers(mdResp.headers);
          const tok = buf ? Math.max(1, Math.floor(buf.byteLength / 4)) : 1;
          applyBaseHeaders(headers, pathname, 200, {
            "Content-Type": "text/markdown; charset=utf-8",
            "x-markdown-tokens": String(tok),
          });
          return new Response(buf, { status: 200, headers });
        }
      }
    }

    const assetResp = await env.ASSETS.fetch(request);
    const headers = new Headers(assetResp.headers);
    applyBaseHeaders(headers, pathname, assetResp.status);
    return new Response(assetResp.body, {
      status: assetResp.status,
      statusText: assetResp.statusText,
      headers,
    });
  },
};
