import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
import type { WorkerConfig } from "../src/ts/worker";

const TESTS_DIR = dirname(fileURLToPath(import.meta.url));
const WORKER_DIST = join(TESTS_DIR, "..", "dist", "worker.js");

interface WorkerModule {
  default: {
    fetch(
      request: Request,
      env: { ASSETS: { fetch(req: Request): Promise<Response> } }
    ): Promise<Response>;
  };
}

const TEST_CFG: WorkerConfig = {
  origin: "https://digest.example.com",
  contentSignal: "ai-train=no, search=yes, ai-input=yes",
  linkHeader:
    '<https://digest.example.com/llms.txt>; rel="describedby"; type="text/markdown"',
  searchEnabled: true,
  globalHeaders: {
    "X-Content-Type-Options": "nosniff",
    "X-Frame-Options": "DENY",
    "Content-Security-Policy": "default-src 'none'",
  },
  cacheRules: [
    ["/*", "public, max-age=300"],
    ["/daily/*", "public, max-age=3600"],
  ],
  redirects: [
    ["/latest", "/daily/2026-11-26", 302],
    ["/weekly/latest", "/weekly/2026-W48", 302],
  ],
};

const SEARCH_INDEX = {
  docs: [
    {
      t: "Fast Transformer Attention Kernel",
      d: "2026-11-26",
      l: "en",
      s: "arXiv",
      g: ["models", "systems"],
      u: "https://digest.example.com/en/daily/2026-11-26#item-1",
      m: "/en/daily/2026-11-26.md",
      p: "New sparse transformer kernel doubles throughput on modern GPUs.",
      lk: ["https://arxiv.org/abs/2611.00001"],
    },
    {
      t: "\u5927\u6a21\u578b\u63a8\u7406\u52a0\u901f\u65b9\u6848",
      d: "2026-11-26",
      l: "zh-CN",
      s: "GitHub",
      g: ["\u63a8\u7406", "LLM"],
      u: "https://digest.example.com/zh/daily/2026-11-26#item-2",
      m: "/zh/daily/2026-11-26.md",
      p: "\u5f00\u6e90\u91cf\u5316\u63a8\u7406\u5f15\u64ce\u5b9e\u73b0\u4e24\u500d\u541e\u5410\u63d0\u5347\u3002",
      lk: ["https://github.com/example/fast-infer"],
    },
  ],
};

function makeAssets(files: Record<string, { body: string; type: string }>) {
  return {
    ASSETS: {
      async fetch(req: Request): Promise<Response> {
        const u = new URL(req.url);
        const hit = files[u.pathname];
        if (!hit) {
          return new Response("Not Found", {
            status: 404,
            headers: { "Content-Type": "text/html; charset=utf-8" },
          });
        }
        return new Response(req.method === "HEAD" ? null : hit.body, {
          status: 200,
          headers: { "Content-Type": hit.type },
        });
      },
    },
  };
}

let workerMod: WorkerModule;

beforeAll(async () => {
  const raw = readFileSync(WORKER_DIST, "utf8");
  expect((raw.match(/__WORKER_CFG_JSON__/g) || []).length).toBe(1);
  const injected = raw.replace("__WORKER_CFG_JSON__", JSON.stringify(TEST_CFG));
  expect(injected).not.toContain("__WORKER_CFG_JSON__");
  const dataUrl =
    "data:text/javascript;base64," + Buffer.from(injected, "utf8").toString("base64");
  workerMod = (await import(dataUrl)) as WorkerModule;
});

describe("built web/dist/worker.js runtime behavior", () => {
  const env = makeAssets({
    "/": { body: "<html>index</html>", type: "text/html; charset=utf-8" },
    "/index.md": { body: "# Archive Index\n\nAll issues.\n", type: "text/markdown" },
    "/daily/2026-11-26": {
      body: "<html><body>Daily HTML</body></html>",
      type: "text/html; charset=utf-8",
    },
    "/daily/2026-11-26.md": {
      body: "# Daily 2026-11-26\n\nMarkdown mirror body.\n",
      type: "text/markdown",
    },
    "/zh/index.md": { body: "# ZH Index\n", type: "text/markdown" },
    "/search-index.json": {
      body: JSON.stringify(SEARCH_INDEX),
      type: "application/json",
    },
  });

  it("serves HTML for Accept: text/html with global + GEO + cache headers", async () => {
    const res = await workerMod.default.fetch(
      new Request("https://digest.example.com/daily/2026-11-26", {
        headers: { Accept: "text/html,application/xhtml+xml,*/*;q=0.8" },
      }),
      env
    );
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toContain("text/html");
    expect(res.headers.get("content-signal")).toBe(TEST_CFG.contentSignal);
    expect(res.headers.get("link")).toBe(TEST_CFG.linkHeader);
    expect(res.headers.get("vary")).toBe("Accept");
    expect(res.headers.get("cache-control")).toBe("public, max-age=3600");
    expect(res.headers.get("x-frame-options")).toBe("DENY");
    expect(await res.text()).toContain("Daily HTML");
  });

  it("negotiates Markdown on Accept: text/markdown and rejects equal q-values", async () => {
    const mdRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/daily/2026-11-26", {
        headers: { Accept: "text/markdown, text/html;q=0.5" },
      }),
      env
    );
    expect(mdRes.status).toBe(200);
    expect(mdRes.headers.get("content-type")).toBe("text/markdown; charset=utf-8");
    expect(Number(mdRes.headers.get("x-markdown-tokens"))).toBeGreaterThanOrEqual(1);
    expect(await mdRes.text()).toContain("Markdown mirror body.");

    // Equal q-values must NOT switch to markdown (strictly mdQ > htmlQ)
    const tieRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/daily/2026-11-26", {
        headers: { Accept: "text/html;q=0.9, text/markdown;q=0.9" },
      }),
      env
    );
    expect(tieRes.status).toBe(200);
    expect(tieRes.headers.get("content-type")).toContain("text/html");
    expect(await tieRes.text()).toContain("Daily HTML");

    // 3-range Accept negotiation: wildcard */* weight > text/markdown must serve HTML
    const wildHtmlRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/daily/2026-11-26", {
        headers: { Accept: "text/markdown;q=0.5, text/html;q=0.4, */*;q=0.8" },
      }),
      env
    );
    expect(wildHtmlRes.status).toBe(200);
    expect(wildHtmlRes.headers.get("content-type")).toContain("text/html");
    expect(await wildHtmlRes.text()).toContain("Daily HTML");

    // 3-range Accept negotiation: text/markdown >= */* and > text/html serves Markdown
    const wildMdRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/daily/2026-11-26", {
        headers: { Accept: "text/markdown;q=0.8, text/html;q=0.4, */*;q=0.8" },
      }),
      env
    );
    expect(wildMdRes.status).toBe(200);
    expect(wildMdRes.headers.get("content-type")).toBe("text/markdown; charset=utf-8");
    expect(await wildMdRes.text()).toContain("Markdown mirror body.");
  });

  it("handles direct .md and /index.md URLs plus HEAD empty body", async () => {
    const idxRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/zh/index.md"),
      env
    );
    expect(idxRes.status).toBe(200);
    expect(idxRes.headers.get("content-type")).toBe("text/markdown; charset=utf-8");
    expect(await idxRes.text()).toBe("# ZH Index\n");

    const headRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/daily/2026-11-26.md", {
        method: "HEAD",
      }),
      env
    );
    expect(headRes.status).toBe(200);
    expect(headRes.headers.get("content-type")).toBe("text/markdown; charset=utf-8");
    expect(await headRes.text()).toBe("");
  });

  it("supports GET, HEAD, and QUERY /.well-known/search with bilingual scoring", async () => {
    const getRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search?q=transformer"),
      env
    );
    expect(getRes.status).toBe(200);
    expect(getRes.headers.get("content-type")).toBe("text/markdown; charset=utf-8");
    expect(getRes.headers.get("accept-query")).toBe("text/plain, application/json");
    expect(Number(getRes.headers.get("x-markdown-tokens"))).toBeGreaterThan(0);
    const body = await getRes.text();
    expect(body).toContain("Fast Transformer Attention Kernel");

    // CJK query
    const zhRes = await workerMod.default.fetch(
      new Request(
        "https://digest.example.com/.well-known/search?q=" +
          encodeURIComponent("\u63a8\u7406\u52a0\u901f")
      ),
      env
    );
    expect(zhRes.status).toBe(200);
    expect(await zhRes.text()).toContain("\u5927\u6a21\u578b\u63a8\u7406\u52a0\u901f\u65b9\u6848");

    // HEAD returns empty body
    const headRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search?q=transformer", {
        method: "HEAD",
      }),
      env
    );
    expect(headRes.status).toBe(200);
    expect(headRes.headers.get("x-markdown-tokens")).toBeTruthy();
    expect(await headRes.text()).toBe("");

    // RFC 10008 QUERY with JSON body
    const queryJson = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search", {
        method: "QUERY",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ q: "transformer", lang: "en", limit: 1 }),
      }),
      env
    );
    expect(queryJson.status).toBe(200);
    const qBody = await queryJson.text();
    expect(qBody).toContain("Fast Transformer Attention Kernel");
    expect(qBody).not.toContain("\u5927\u6a21\u578b");

    // QUERY with text/plain body
    const queryPlain = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search", {
        method: "QUERY",
        headers: { "Content-Type": "text/plain" },
        body: "sparse kernel",
      }),
      env
    );
    expect(queryPlain.status).toBe(200);
    expect(await queryPlain.text()).toContain("Fast Transformer Attention Kernel");
  });

  it("enforces 400, 405, 413, and 503 search edge cases", async () => {
    const emptyRes = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search"),
      env
    );
    expect(emptyRes.status).toBe(400);

    const badMethod = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search", {
        method: "POST",
        body: "q=test",
      }),
      env
    );
    expect(badMethod.status).toBe(405);
    expect(badMethod.headers.get("allow")).toBe("GET, HEAD, QUERY");

    const tooLarge = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search", {
        method: "QUERY",
        headers: { "Content-Type": "text/plain" },
        body: "a".repeat(4097),
      }),
      env
    );
    expect(tooLarge.status).toBe(413);

    // Streaming 4097-byte body must abort early without pulling subsequent chunks
    let pulledThirdChunk = false;
    let chunkIdx = 0;
    const chunks = [
      new Uint8Array(2048).fill(0x61),
      new Uint8Array(2049).fill(0x62),
      new Uint8Array(2048).fill(0x63),
    ];
    const stream = new ReadableStream<Uint8Array>({
      pull(controller) {
        if (chunkIdx === 2) pulledThirdChunk = true;
        if (chunkIdx < chunks.length) {
          controller.enqueue(chunks[chunkIdx++]);
        } else {
          controller.close();
        }
      },
    });
    const streamInit = {
      method: "QUERY",
      headers: { "Content-Type": "text/plain" },
      body: stream,
      duplex: "half",
    } as RequestInit;
    const streamTooLarge = await workerMod.default.fetch(
      new Request("https://digest.example.com/.well-known/search", streamInit),
      env
    );
    expect(streamTooLarge.status).toBe(413);
    expect(pulledThirdChunk).toBe(false);

    // Missing search index -> 503 (use distinct origin so cached index doesn't interfere)
    const noIdxEnv = makeAssets({});
    const unavailable = await workerMod.default.fetch(
      new Request("https://missing-idx.example.com/.well-known/search?q=test"),
      noIdxEnv
    );
    expect(unavailable.status).toBe(503);
    expect(unavailable.headers.get("retry-after")).toBe("60");

    const unavailableHead = await workerMod.default.fetch(
      new Request("https://missing-idx.example.com/.well-known/search?q=test", {
        method: "HEAD",
      }),
      noIdxEnv
    );
    expect(unavailableHead.status).toBe(503);
    expect(await unavailableHead.text()).toBe("");
  });

  it("applies static redirects and 404 noindex / no-store headers", async () => {
    const redir = await workerMod.default.fetch(
      new Request("https://digest.example.com/latest"),
      env
    );
    expect(redir.status).toBe(302);
    expect(redir.headers.get("location")).toBe("https://digest.example.com/daily/2026-11-26");

    const notFound = await workerMod.default.fetch(
      new Request("https://digest.example.com/does-not-exist"),
      env
    );
    expect(notFound.status).toBe(404);
    expect(notFound.headers.get("x-robots-tag")).toBe("noindex, nofollow");
    expect(notFound.headers.get("cache-control")).toBe("no-store");
  });

  it("localizes rel=alternate markdown Link header per page with safe fallback", async () => {
    const raw = readFileSync(WORKER_DIST, "utf8");
    const localizedCfg: WorkerConfig = {
      ...TEST_CFG,
      linkHeader:
        '</llms.txt>; rel="describedby"; type="text/markdown", </en/index.md>; rel="alternate"; type="text/markdown", </sitemap.xml>; rel="sitemap"; type="application/xml"',
    };
    const injected = raw.replace("__WORKER_CFG_JSON__", JSON.stringify(localizedCfg));
    const dataUrl =
      "data:text/javascript;base64," + Buffer.from(injected, "utf8").toString("base64");
    const localizedWorker = (await import(dataUrl)) as WorkerModule;
    const demoEnv = makeAssets({
      "/en/": { body: "<html>EN</html>", type: "text/html; charset=utf-8" },
      "/en/index.md": { body: "# EN Index\n", type: "text/markdown" },
      "/zh-CN/": { body: "<html>ZH</html>", type: "text/html; charset=utf-8" },
      "/zh-CN/index.md": { body: "# ZH Index\n", type: "text/markdown" },
      "/en/daily/latest": { body: "<html>Latest</html>", type: "text/html; charset=utf-8" },
      "/en/daily/latest.md": { body: "# Latest EN\n", type: "text/markdown" },
    });

    const enRes = await localizedWorker.default.fetch(
      new Request("https://demo.example.com/en/"),
      demoEnv
    );
    expect(enRes.headers.get("link")).toContain(
      '</en/index.md>; rel="alternate"; type="text/markdown"'
    );

    const zhRes = await localizedWorker.default.fetch(
      new Request("https://demo.example.com/zh-CN/"),
      demoEnv
    );
    expect(zhRes.headers.get("link")).toContain(
      '</zh-CN/index.md>; rel="alternate"; type="text/markdown"'
    );

    const latestHead = await localizedWorker.default.fetch(
      new Request("https://demo.example.com/en/daily/latest", { method: "HEAD" }),
      demoEnv
    );
    expect(latestHead.headers.get("link")).toContain(
      '</en/daily/latest.md>; rel="alternate"; type="text/markdown"'
    );

    const missing = await localizedWorker.default.fetch(
      new Request("https://demo.example.com/missing"),
      demoEnv
    );
    expect(missing.status).toBe(404);
    expect(missing.headers.get("link")).toContain(
      '</en/index.md>; rel="alternate"; type="text/markdown"'
    );
  });

  it("applies global security headers on /.well-known/search and 302 redirects and escapes reflected query markdown", async () => {
    const search200 = await workerMod.default.fetch(
      new Request(
        "https://digest.example.com/.well-known/search?q=" +
          encodeURIComponent("[click](https://evil.example)<script>x</script>")
      ),
      env
    );
    expect(search200.status).toBe(200);
    expect(search200.headers.get("content-security-policy")).toBe("default-src 'none'");
    expect(search200.headers.get("x-frame-options")).toBe("DENY");
    expect(search200.headers.get("x-content-type-options")).toBe("nosniff");
    const md = await search200.text();
    expect(md).not.toContain("[click](");
    expect(md).not.toContain("<script>");

    for (const req of [
      new Request("https://digest.example.com/.well-known/search"),
      new Request("https://digest.example.com/.well-known/search", { method: "POST" }),
      new Request("https://digest.example.com/latest"),
    ]) {
      const res = await workerMod.default.fetch(req, env);
      expect(res.headers.get("content-security-policy")).toBe("default-src 'none'");
      expect(res.headers.get("x-frame-options")).toBe("DENY");
      expect(res.headers.get("x-content-type-options")).toBe("nosniff");
    }
  });
});
