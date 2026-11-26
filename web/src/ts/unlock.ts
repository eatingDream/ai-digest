interface CipherEnvelope {
  v: number;
  z?: string;
  iter: number;
  salt: string;
  iv: string;
  ct: string;
}

interface UnlockStrings {
  pw_open?: string;
  pw_opening?: string;
  pw_unlocking?: string;
  pw_wrong?: string;
  pw_unsupported?: string;
}

interface StoredKeyRecord {
  v: number;
  salt: string;
  key: string;
}

const STORE_KEY = "aie.k";
const EXPECTED_ITER = 600000;
const EXPECTED_SALT_LEN = 16;
const EXPECTED_IV_LEN = 12;
const MAX_DECOMPRESSED_BYTES = 32 * 1024 * 1024;

function b64ToBytes(s: string): Uint8Array<ArrayBuffer> {
  const bin = atob(s);
  const buf = new ArrayBuffer(bin.length);
  const out = new Uint8Array(buf);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

async function gunzipBounded(
  data: ArrayBuffer,
  maxBytes = MAX_DECOMPRESSED_BYTES
): Promise<Uint8Array<ArrayBuffer>> {
  const stream = new Blob([data])
    .stream()
    .pipeThrough(new DecompressionStream("gzip"));
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value || value.byteLength === 0) continue;
      total += value.byteLength;
      if (total > maxBytes) {
        await reader.cancel("decompressed payload too large").catch(() => {});
        throw new Error("decompressed payload too large");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }
  const buf = new ArrayBuffer(total);
  const out = new Uint8Array(buf);
  let offset = 0;
  for (const c of chunks) {
    out.set(c, offset);
    offset += c.byteLength;
  }
  return out;
}

function bytesToB64(u: Uint8Array): string {
  let s = "";
  const chunk = 0x8000;
  for (let i = 0; i < u.length; i += chunk) {
    s += String.fromCharCode(...u.subarray(i, i + chunk));
  }
  return btoa(s);
}

function scrollToHash(h: string): void {
  if (!h || h.length < 2) return;
  let id = h.slice(1);
  try {
    id = decodeURIComponent(id);
  } catch {
    // Keep raw fragment.
  }
  const el = document.getElementById(id) || document.getElementsByName(id)[0];
  if (el && "scrollIntoView" in el) el.scrollIntoView();
}

function openPage(html: string): void {
  const hash = window.location.hash;
  document.open("text/html");
  document.write(html);
  document.close();
  if (hash && window.location.hash !== hash) {
    try {
      window.history.replaceState(null, "", hash);
    } catch {
      // Ignore history restrictions.
    }
  }
  scrollToHash(hash);
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => scrollToHash(hash));
  }
}

function initUnlock(): void {
  const boxEl = document.getElementById("box");
  const form =
    (document.getElementById("pwForm") as HTMLFormElement | null) ||
    (document.getElementById("f") as HTMLFormElement | null);
  const pwInput = document.getElementById("pw") as HTMLInputElement | null;
  const rmInput = document.getElementById("rm") as HTMLInputElement | null;
  const goBtn = document.getElementById("go") as HTMLButtonElement | null;
  const goLbl =
    goBtn?.querySelector<HTMLElement>(".go-lbl, .go-t") ?? null;
  const spinEl = goBtn?.querySelector<HTMLElement>(".spin") ?? null;
  const errEl =
    document.querySelector<HTMLElement>("p.err") ||
    document.getElementById("err");

  let strings: UnlockStrings = {};
  try {
    const cfgEl = document.getElementById("cfg");
    if (cfgEl?.textContent) {
      strings = JSON.parse(cfgEl.textContent) as UnlockStrings;
    }
  } catch {
    strings = {};
  }

  const txtOpen = strings.pw_open || goLbl?.textContent?.trim() || "Open";
  const txtOpening = strings.pw_opening || strings.pw_unlocking || "Opening...";
  const txtWrong = strings.pw_wrong || "Incorrect passphrase";
  const txtUnsupported =
    strings.pw_unsupported || "Browser does not support WebCrypto decryption.";

  const showForm = (msg?: string): void => {
    if (boxEl) boxEl.hidden = false;
    if (errEl) {
      if (msg) {
        errEl.textContent = msg;
        errEl.hidden = false;
      }
    }
    pwInput?.focus();
  };

  let env: CipherEnvelope;
  try {
    const cEl = document.getElementById("c");
    if (!cEl?.textContent) throw new Error("missing envelope");
    env = JSON.parse(cEl.textContent) as CipherEnvelope;
  } catch {
    showForm(txtUnsupported);
    return;
  }

  let saltBytes: Uint8Array<ArrayBuffer>;
  let ivBytes: Uint8Array<ArrayBuffer>;
  let ctBytes: Uint8Array<ArrayBuffer>;
  try {
    if (typeof env.salt !== "string" || typeof env.iv !== "string" || typeof env.ct !== "string") {
      throw new Error("invalid envelope fields");
    }
    saltBytes = b64ToBytes(env.salt);
    ivBytes = b64ToBytes(env.iv);
    ctBytes = b64ToBytes(env.ct);
  } catch {
    showForm(txtUnsupported);
    return;
  }

  const unsupported =
    !window.crypto ||
    !window.crypto.subtle ||
    (env.v !== 1 && env.v !== 2) ||
    typeof env.iter !== "number" ||
    env.iter !== EXPECTED_ITER ||
    saltBytes.byteLength !== EXPECTED_SALT_LEN ||
    ivBytes.byteLength !== EXPECTED_IV_LEN ||
    ctBytes.byteLength === 0 ||
    (env.v === 1 && env.z !== undefined) ||
    (env.v === 2 &&
      (env.z !== "gzip" || typeof DecompressionStream === "undefined"));

  if (unsupported) {
    showForm(txtUnsupported);
    return;
  }

  const loadStored = (): StoredKeyRecord | null => {
    const stores: Storage[] = [];
    try {
      stores.push(window.sessionStorage);
    } catch {
      // Ignore storage restrictions.
    }
    try {
      stores.push(window.localStorage);
    } catch {
      // Ignore storage restrictions.
    }
    for (const st of stores) {
      try {
        const raw = st.getItem(STORE_KEY);
        if (!raw) continue;
        const rec = JSON.parse(raw) as StoredKeyRecord;
        if (rec && rec.v === 1 && rec.salt === env.salt && rec.key) return rec;
      } catch {
        // Ignore malformed entry.
      }
    }
    return null;
  };

  const saveKey = (keyBytes: Uint8Array, remember: boolean): void => {
    const rec = JSON.stringify({
      v: 1,
      salt: env.salt,
      key: bytesToB64(keyBytes),
    });
    try {
      window.sessionStorage.setItem(STORE_KEY, rec);
    } catch {
      // Ignore.
    }
    try {
      if (remember) window.localStorage.setItem(STORE_KEY, rec);
      else window.localStorage.removeItem(STORE_KEY);
    } catch {
      // Ignore.
    }
  };

  const clearStored = (): void => {
    try {
      window.sessionStorage.removeItem(STORE_KEY);
    } catch {
      // Ignore.
    }
    try {
      window.localStorage.removeItem(STORE_KEY);
    } catch {
      // Ignore.
    }
  };

  const decryptWithKey = async (keyBytes: Uint8Array<ArrayBuffer>): Promise<string> => {
    const key = await window.crypto.subtle.importKey(
      "raw",
      keyBytes,
      "AES-GCM",
      false,
      ["decrypt"]
    );
    const ptBuf = await window.crypto.subtle.decrypt(
      { name: "AES-GCM", iv: ivBytes },
      key,
      ctBytes
    );
    if (env.v === 2) {
      if (env.z !== "gzip") throw new Error("unexpected compression");
      const out = await gunzipBounded(ptBuf, MAX_DECOMPRESSED_BYTES);
      return new TextDecoder("utf-8").decode(out);
    }
    if (ptBuf.byteLength > MAX_DECOMPRESSED_BYTES) {
      throw new Error("payload too large");
    }
    return new TextDecoder("utf-8").decode(ptBuf);
  };

  const derive = async (password: string): Promise<Uint8Array<ArrayBuffer>> => {
    const base = await window.crypto.subtle.importKey(
      "raw",
      new TextEncoder().encode(password),
      "PBKDF2",
      false,
      ["deriveBits"]
    );
    const bits = await window.crypto.subtle.deriveBits(
      {
        name: "PBKDF2",
        salt: saltBytes,
        iterations: EXPECTED_ITER,
        hash: "SHA-256",
      },
      base,
      256
    );
    return new Uint8Array(bits);
  };

  form?.addEventListener("submit", async (e: SubmitEvent) => {
    e.preventDefault();
    if (!pwInput || !goBtn) return;
    if (errEl) errEl.textContent = "";
    goBtn.disabled = true;
    goBtn.classList.add("is-loading");
    if (goLbl) goLbl.textContent = txtOpening;
    if (spinEl) spinEl.hidden = false;

    try {
      const key = await derive(pwInput.value);
      const html = await decryptWithKey(key);
      saveKey(key, Boolean(rmInput?.checked));
      openPage(html);
    } catch {
      if (errEl) {
        errEl.textContent = txtWrong;
        errEl.hidden = false;
      }
      goBtn.disabled = false;
      goBtn.classList.remove("is-loading");
      if (goLbl) goLbl.textContent = txtOpen;
      if (spinEl) spinEl.hidden = true;
      if (boxEl) {
        boxEl.classList.remove("is-shake");
        void boxEl.offsetWidth;
        boxEl.classList.add("is-shake");
      }
      pwInput.focus();
      pwInput.select();
    }
  });

  const rec = loadStored();
  if (rec) {
    decryptWithKey(b64ToBytes(rec.key))
      .then((html) => openPage(html))
      .catch(() => {
        clearStored();
        showForm();
      });
  } else {
    showForm();
  }
}

initUnlock();
