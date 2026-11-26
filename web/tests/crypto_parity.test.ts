import { execFileSync, spawnSync } from "node:child_process";
import { webcrypto } from "node:crypto";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { describe, expect, it } from "vitest";

const TESTS_DIR = dirname(fileURLToPath(import.meta.url));
const VERIFY_CLI = join(TESTS_DIR, "..", "dist", "verify_decrypt.mjs");

// Fixed deterministic test vectors (16-byte salt, 12-byte IV)
const FIXED_SALT = Uint8Array.from([
  0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
  0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
]);
const FIXED_IV = Uint8Array.from([
  0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
  0x18, 0x19, 0x1a, 0x1b,
]);

interface Envelope {
  v: number;
  z?: string;
  iter: number;
  salt: string;
  iv: string;
  ct: string;
}

async function deriveKey(
  password: string,
  salt: Uint8Array<ArrayBuffer>,
  iter = 600000
): Promise<Uint8Array<ArrayBuffer>> {
  const base = await webcrypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(password),
    "PBKDF2",
    false,
    ["deriveBits"]
  );
  const bits = await webcrypto.subtle.deriveBits(
    { name: "PBKDF2", salt, iterations: iter, hash: "SHA-256" },
    base,
    256
  );
  return new Uint8Array(bits);
}

async function encryptEnvelope(
  plaintext: string,
  password: string,
  version: 1 | 2
): Promise<Envelope> {
  const keyBytes = await deriveKey(password, FIXED_SALT, 600000);
  const aesKey = await webcrypto.subtle.importKey(
    "raw",
    keyBytes,
    "AES-GCM",
    false,
    ["encrypt"]
  );
  const rawBytes = new TextEncoder().encode(plaintext);
  const payload = version === 2 ? Uint8Array.from(gzipSync(rawBytes, { level: 9 })) : rawBytes;
  const ctBuf = await webcrypto.subtle.encrypt(
    { name: "AES-GCM", iv: FIXED_IV },
    aesKey,
    payload
  );
  const env: Envelope = {
    v: version,
    iter: 600000,
    salt: Buffer.from(FIXED_SALT).toString("base64"),
    iv: Buffer.from(FIXED_IV).toString("base64"),
    ct: Buffer.from(ctBuf).toString("base64"),
  };
  if (version === 2) env.z = "gzip";
  return env;
}

async function decryptEnvelope(env: Envelope, password: string): Promise<string> {
  if (env.v !== 1 && env.v !== 2) throw new Error(`unexpected version ${env.v}`);
  if (env.iter < 600000) throw new Error(`iter too low ${env.iter}`);
  const salt = Uint8Array.from(Buffer.from(env.salt, "base64"));
  const keyBytes = await deriveKey(password, salt, env.iter);
  const aesKey = await webcrypto.subtle.importKey(
    "raw",
    keyBytes,
    "AES-GCM",
    false,
    ["decrypt"]
  );
  let pt = await webcrypto.subtle.decrypt(
    { name: "AES-GCM", iv: Uint8Array.from(Buffer.from(env.iv, "base64")) },
    aesKey,
    Uint8Array.from(Buffer.from(env.ct, "base64"))
  );
  if (env.v === 2) {
    if (env.z !== "gzip") throw new Error(`unexpected compression ${env.z}`);
    const stream = new Blob([pt])
      .stream()
      .pipeThrough(new DecompressionStream("gzip"));
    pt = await new Response(stream).arrayBuffer();
  }
  return new TextDecoder("utf-8").decode(pt);
}

function wrapLockedHtml(env: Envelope): string {
  return `<!DOCTYPE html><html><body><main id="box"><input id="pw" type="password"><input id="rm" type="checkbox"></main><script type="application/json" id="c">${JSON.stringify(env)}</script></body></html>`;
}

describe("PBKDF2-SHA256 + AES-256-GCM + gzip deterministic crypto parity", () => {
  it("round-trips v2 gzip and v1 uncompressed deterministic vectors", async () => {
    const pw = "test-passphrase-2026-deterministic";
    const html = "<!DOCTYPE html><html><body><h1>AI Digest Test</h1><p>Hello world</p></body></html>";

    const envV2 = await encryptEnvelope(html, pw, 2);
    expect(envV2.v).toBe(2);
    expect(envV2.z).toBe("gzip");
    expect(await decryptEnvelope(envV2, pw)).toBe(html);

    const envV1 = await encryptEnvelope(html, pw, 1);
    expect(envV1.v).toBe(1);
    expect(envV1.z).toBeUndefined();
    expect(await decryptEnvelope(envV1, pw)).toBe(html);

    // Wrong password must fail authentication
    await expect(decryptEnvelope(envV2, "wrong-passphrase-2026")).rejects.toThrow();
  });

  it("handles UTF-8/CJK passphrases and preserves leading/trailing spaces", async () => {
    const cjkPw = "\u53e3\u4ee4\u6d4b\u8bd5-\u65e5\u672c\u8a9e-\u03b1\u03b2\u03b3-2026";
    const spacedPw = "  leading and trailing spaces  ";
    const sample = "<p>\u4e2d\u6587\u5185\u5bb9 & math $E=mc^2$</p>";

    const cjkEnv = await encryptEnvelope(sample, cjkPw, 2);
    expect(await decryptEnvelope(cjkEnv, cjkPw)).toBe(sample);

    const spacedEnv = await encryptEnvelope(sample, spacedPw, 2);
    expect(await decryptEnvelope(spacedEnv, spacedPw)).toBe(sample);
    await expect(decryptEnvelope(spacedEnv, spacedPw.trim())).rejects.toThrow();
  });

  it("executes built web/dist/verify_decrypt.mjs CLI for --version, --help, ENCRYPT=1 (v1+v2), and ENCRYPT=0", async () => {
    const pkgVersion = JSON.parse(
      readFileSync(join(TESTS_DIR, "..", "package.json"), "utf8")
    ).version;
    const ver = execFileSync(process.execPath, [VERIFY_CLI, "--version"], {
      encoding: "utf8",
    }).trim();
    expect(ver).toBe(`ai-digest ${pkgVersion}`);

    const help = execFileSync(process.execPath, [VERIFY_CLI, "--help"], {
      encoding: "utf8",
    });
    expect(help).toContain("usage: verify_decrypt.mjs");

    const tmp = mkdtempSync(join(tmpdir(), "ai-digest-verify-"));
    try {
      const pw = "  cjk-\u53e3\u4ee4-passphrase-2026  ";
      const pwFile = join(tmp, "site-password");
      // Trailing \n only; leading/trailing spaces must be preserved!
      writeFileSync(pwFile, `${pw}\n`, { encoding: "utf8", mode: 0o600 });

      mkdirSync(join(tmp, "site", "plain", "daily"), { recursive: true });
      mkdirSync(join(tmp, "site", "public", "daily"), { recursive: true });
      writeFileSync(join(tmp, "site", "CONFIG"), "SLUG=\nENCRYPT=1\n", "utf8");

      const plainV2 = "<!DOCTYPE html><html><body>v2 gzip \u4e2d\u6587</body></html>";
      const plainV1 = "<!DOCTYPE html><html><body>v1 plain text</body></html>";
      const envV2 = await encryptEnvelope(plainV2, pw, 2);
      const envV1 = await encryptEnvelope(plainV1, pw, 1);

      writeFileSync(join(tmp, "site", "plain", "index.html"), plainV2, "utf8");
      writeFileSync(join(tmp, "site", "public", "index.html"), wrapLockedHtml(envV2), "utf8");
      writeFileSync(join(tmp, "site", "plain", "daily", "2026-11-26.html"), plainV1, "utf8");
      writeFileSync(
        join(tmp, "site", "public", "daily", "2026-11-26.html"),
        wrapLockedHtml(envV1),
        "utf8"
      );

      const outEnc = execFileSync(process.execPath, [VERIFY_CLI, "--local"], {
        encoding: "utf8",
        env: {
          ...process.env,
          DIGEST_ROOT: tmp,
          DIGEST_SITE_PASSWORD_FILE: pwFile,
        },
      });
      expect(outEnc).toContain("all 2 page(s) decrypted and matched plaintext");

      // ENCRYPT=0 parity
      writeFileSync(join(tmp, "site", "CONFIG"), "SLUG=\nENCRYPT=0\n", "utf8");
      writeFileSync(join(tmp, "site", "public", "index.html"), plainV2, "utf8");
      writeFileSync(join(tmp, "site", "public", "daily", "2026-11-26.html"), plainV1, "utf8");

      const outPub = execFileSync(process.execPath, [VERIFY_CLI, "--local"], {
        encoding: "utf8",
        env: {
          ...process.env,
          DIGEST_ROOT: tmp,
        },
      });
      expect(outPub).toContain("all 2 page(s) matched plaintext");
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("handles 404.html as unencrypted control page with SLUG='' + ENCRYPT=1 and rejects 404-only sites", async () => {
    const tmp = mkdtempSync(join(tmpdir(), "ai-digest-404-"));
    try {
      const pw = "control-404-passphrase-2026";
      const pwFile = join(tmp, "site-password");
      writeFileSync(pwFile, `${pw}\n`, { encoding: "utf8", mode: 0o600 });

      mkdirSync(join(tmp, "site", "plain"), { recursive: true });
      mkdirSync(join(tmp, "site", "public"), { recursive: true });
      writeFileSync(join(tmp, "site", "CONFIG"), "SLUG=\nENCRYPT=1\n", "utf8");

      const html404 = "<!DOCTYPE html><html><body><main class=\"nf\"><h1>404</h1></main></body></html>";
      writeFileSync(join(tmp, "site", "plain", "404.html"), html404, "utf8");
      writeFileSync(join(tmp, "site", "public", "404.html"), html404, "utf8");

      // 404.html alone must be rejected
      const only404 = spawnSync(process.execPath, [VERIFY_CLI, "--local"], {
        encoding: "utf8",
        env: {
          ...process.env,
          DIGEST_ROOT: tmp,
          DIGEST_SITE_PASSWORD_FILE: pwFile,
        },
      });
      expect(only404.status).not.toBe(0);
      expect(only404.stderr).toContain("404.html");

      // With an encrypted index.html alongside plaintext 404.html, --local succeeds
      const plainIdx = "<!DOCTYPE html><html><body><h1>Encrypted Index</h1></body></html>";
      const envIdx = await encryptEnvelope(plainIdx, pw, 2);
      writeFileSync(join(tmp, "site", "plain", "index.html"), plainIdx, "utf8");
      writeFileSync(join(tmp, "site", "public", "index.html"), wrapLockedHtml(envIdx), "utf8");

      const ok = execFileSync(process.execPath, [VERIFY_CLI, "--local"], {
        encoding: "utf8",
        env: {
          ...process.env,
          DIGEST_ROOT: tmp,
          DIGEST_SITE_PASSWORD_FILE: pwFile,
        },
      });
      expect(ok).toContain("OK  local 404.html");
      expect(ok).toContain("OK  local index.html");
      expect(ok).toContain("all 2 page(s) decrypted and matched plaintext");

      // Byte-for-byte 404.html check must detect raw byte differences that both decode to U+FFFD
      writeFileSync(join(tmp, "site", "plain", "404.html"), Buffer.from([0x3c, 0x70, 0x3e, 0xfe, 0x3c, 0x2f, 0x70, 0x3e]));
      writeFileSync(join(tmp, "site", "public", "404.html"), Buffer.from([0x3c, 0x70, 0x3e, 0xff, 0x3c, 0x2f, 0x70, 0x3e]));
      const byteDiff = spawnSync(process.execPath, [VERIFY_CLI, "--local"], {
        encoding: "utf8",
        env: {
          ...process.env,
          DIGEST_ROOT: tmp,
          DIGEST_SITE_PASSWORD_FILE: pwFile,
        },
      });
      expect(byteDiff.status).not.toBe(0);
      expect(byteDiff.stderr).toContain("first diff at 3");
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("rejects non-600000 PBKDF2 iterations and wrong salt/IV lengths before key derivation", async () => {
    const tmp = mkdtempSync(join(tmpdir(), "ai-digest-env-limits-"));
    try {
      const pw = "envelope-limits-passphrase-2026";
      const pwFile = join(tmp, "site-password");
      writeFileSync(pwFile, `${pw}\n`, { encoding: "utf8", mode: 0o600 });

      mkdirSync(join(tmp, "site", "plain"), { recursive: true });
      mkdirSync(join(tmp, "site", "public"), { recursive: true });
      writeFileSync(join(tmp, "site", "CONFIG"), "SLUG=\nENCRYPT=1\n", "utf8");

      const plain = "<!DOCTYPE html><html><body>ok</body></html>";
      const validEnv = await encryptEnvelope(plain, pw, 2);
      writeFileSync(join(tmp, "site", "plain", "index.html"), plain, "utf8");

      const badCases: Array<{ label: string; env: Envelope; expectErr: RegExp }> = [
        {
          label: "iter too low",
          env: { ...validEnv, iter: 599999 },
          expectErr: /600000/,
        },
        {
          label: "iter too high (DoS)",
          env: { ...validEnv, iter: 50000000 },
          expectErr: /600000/,
        },
        {
          label: "wrong salt length",
          env: { ...validEnv, salt: Buffer.alloc(8, 1).toString("base64") },
          expectErr: /salt length/i,
        },
        {
          label: "wrong iv length",
          env: { ...validEnv, iv: Buffer.alloc(16, 2).toString("base64") },
          expectErr: /iv length/i,
        },
        {
          label: "v1 envelope with smuggled z field",
          env: { ...validEnv, v: 1, z: "gzip" },
          expectErr: /compression.*v1/i,
        },
      ];

      for (const tc of badCases) {
        writeFileSync(
          join(tmp, "site", "public", "index.html"),
          wrapLockedHtml(tc.env),
          "utf8"
        );
        const res = spawnSync(process.execPath, [VERIFY_CLI, "--local"], {
          encoding: "utf8",
          env: {
            ...process.env,
            DIGEST_ROOT: tmp,
            DIGEST_SITE_PASSWORD_FILE: pwFile,
          },
        });
        expect(res.status, tc.label).not.toBe(0);
        expect(res.stderr, tc.label).toMatch(tc.expectErr);
      }

      // Built browser unlock.js also enforces strict 600000, salt=16, iv=12, and 32 MiB cap
      const unlockJs = readFileSync(join(TESTS_DIR, "..", "dist", "unlock.js"), "utf8");
      expect(unlockJs).toContain("6e5");
      expect(unlockJs).toContain("33554432");
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("blocks SSRF targets and redacts credentials/query strings in verify_decrypt.mjs logs", () => {
    const tmp = mkdtempSync(join(tmpdir(), "ai-digest-ssrf-"));
    try {
      mkdirSync(join(tmp, "site", "plain"), { recursive: true });
      writeFileSync(join(tmp, "site", "CONFIG"), "SLUG=\nENCRYPT=0\n", "utf8");
      const plainFile = join(tmp, "site", "plain", "index.html");
      writeFileSync(plainFile, "<html></html>", "utf8");

      const blockedUrls = [
        "http://127.0.0.1:8080/index",
        "http://localhost/index",
        "http://service.internal/index",
        "http://169.254.169.254/latest/meta-data",
        "http://[::ffff:127.0.0.1]/index",
        "file:///etc/passwd",
        "https://secretuser:secretpass@127.0.0.1/page?token=supersecret",
      ];

      for (const u of blockedUrls) {
        const res = spawnSync(
          process.execPath,
          [VERIFY_CLI, "--url", u, "--plain", plainFile],
          {
            encoding: "utf8",
            env: {
              ...process.env,
              DIGEST_ROOT: tmp,
            },
          }
        );
        expect(res.status, `should block ${u}`).not.toBe(0);
        const combined = `${res.stdout}\n${res.stderr}`;
        expect(combined).not.toContain("secretuser");
        expect(combined).not.toContain("secretpass");
        expect(combined).not.toContain("supersecret");
      }
      // Built verify_decrypt.mjs uses a dedicated direct non-proxying Agent (never globalAgent/env proxy)
      const builtVerify = readFileSync(VERIFY_CLI, "utf8");
      expect(builtVerify).toMatch(/new \w+\.Agent\(\{keepAlive:!1\}\):new \w+\.Agent\(\{keepAlive:!1\}\)/);
      expect(builtVerify).toMatch(/agent:\w+,timeout:/);
    } finally {
      rmSync(tmp, { recursive: true, force: true });
    }
  });
});
