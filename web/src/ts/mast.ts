export type MastState = "shown" | "hidden";
export type ThemeMode = "system" | "light" | "dark";

export interface MastController {
  setPinned(key: string, pinned: boolean): void;
  setState(next: MastState): void;
  setProgrammaticScroll(active: boolean): void;
  getMastHeight(): number;
  getMastOffset(): number;
  isReducedMotion(): boolean;
  isZh(): boolean;
  isTypingTarget(el: EventTarget | null): boolean;
  isPinned(): boolean;
  syncLang(): void;
}

const THEME_KEY = "digest-theme";
export const LANG_ANCHOR_KEY = "ai_digest_lang_anchor";
const THEME_MODES: readonly ThemeMode[] = ["system", "light", "dark"];

export interface LangAnchorSnapshot {
  anchorId: string;
  relTop: number;
  ts: number;
}

export function normalizeAnchorId(id: string): string {
  return id.startsWith("en-") ? id.slice(3) : id;
}

export function captureVisibleAnchor(
  scope: ParentNode = document,
  mastHeight = 56
): LangAnchorSnapshot {
  if (window.scrollY < 40) {
    return { anchorId: "__top__", relTop: 0, ts: Date.now() };
  }
  const refY = mastHeight + 16;
  const pickNearest = (selector: string): HTMLElement | null => {
    const list = Array.from(scope.querySelectorAll<HTMLElement>(selector)).filter(
      (el) => !el.hidden && el.offsetParent !== null
    );
    let best: HTMLElement | null = null;
    let bestDist = Infinity;
    for (const el of list) {
      const r = el.getBoundingClientRect();
      if (r.height <= 0 || r.bottom < mastHeight) continue;
      const dist = Math.abs(r.top - refY);
      if (dist < bestDist) {
        bestDist = dist;
        best = el;
      }
      if (r.top > refY + 240) break;
    }
    return best;
  };
  const best =
    pickNearest("article.story[id], section.front[id]") ||
    pickNearest("section.sec[id]");
  if (!best) {
    return { anchorId: "__top__", relTop: 0, ts: Date.now() };
  }
  return {
    anchorId: normalizeAnchorId(best.id),
    relTop: Math.round(best.getBoundingClientRect().top),
    ts: Date.now(),
  };
}

const THEME_SVGS: Record<ThemeMode, string> = {
  system:
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><circle cx="8" cy="8" r="5.5"/><path d="M8 2.5a5.5 5.5 0 0 1 0 11z" fill="currentColor" stroke="none"/></svg>',
  light:
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true"><circle cx="8" cy="8" r="3"/><path d="M8 1.5v1.5M8 13v1.5M1.5 8H3M13 8h1.5M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M12.6 3.4l-1.1 1.1M4.5 11.5l-1.1 1.1"/></svg>',
  dark:
    '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" aria-hidden="true"><path d="M13 9.7A5.4 5.4 0 0 1 6.3 3a5.5 5.5 0 1 0 6.7 6.7Z"/></svg>',
};

function isTypingTarget(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    el.isContentEditable
  );
}

export function initMast(): MastController {
  const root = document.documentElement;
  const masts = Array.from(document.querySelectorAll<HTMLElement>("header.mast"));
  const isZh = (): boolean => (root.lang || "").toLowerCase().startsWith("zh");

  const rmQuery =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)")
      : null;
  const fineQuery =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(pointer: fine)")
      : null;
  const darkQuery =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;

  let reducedMotion = Boolean(rmQuery?.matches);
  let state: MastState = "shown";
  let mastH = 56;
  let hovering = false;
  let focusWithin = false;
  let programmaticScroll = false;
  const pinReasons = new Set<string>();

  const isPinned = (): boolean =>
    reducedMotion || hovering || focusWithin || pinReasons.size > 0;

  const syncCssVars = (): void => {
    const offset = state === "shown" ? mastH : 0;
    root.dataset.mast = state;
    root.style.setProperty("--mast-h", `${mastH}px`);
    root.style.setProperty("--mast-offset", `${offset}px`);
  };

  let lastY = window.scrollY;
  let acc = 0;

  const setState = (next: MastState): void => {
    const target: MastState = next === "hidden" && isPinned() ? "shown" : next;
    lastY = window.scrollY;
    acc = 0;
    if (target === state && root.dataset.mast === target) return;
    state = target;
    syncCssVars();
  };

  const setPinned = (key: string, pinned: boolean): void => {
    if (pinned) {
      pinReasons.add(key);
      setState("shown");
    } else {
      pinReasons.delete(key);
    }
  };

  const setProgrammaticScroll = (active: boolean): void => {
    programmaticScroll = active;
    lastY = window.scrollY;
    acc = 0;
  };

  const measureMast = (): void => {
    for (const mast of masts) {
      const row = mast.querySelector<HTMLElement>(".mast-row");
      const rawH = row
        ? row.getBoundingClientRect().height + 1
        : mast.getBoundingClientRect().height;
      const h = Math.round(rawH);
      if (h > 0) {
        if (h !== mastH) {
          mastH = h;
          syncCssVars();
        }
        return;
      }
    }
  };

  if (masts.length > 0) {
    measureMast();
    const ro =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(measureMast)
        : null;
    for (const mast of masts) {
      const row = mast.querySelector<HTMLElement>(".mast-row") ?? mast;
      ro?.observe(row);
      mast.addEventListener("mouseenter", () => {
        hovering = true;
        setState("shown");
      });
      mast.addEventListener("mouseleave", () => {
        hovering = false;
      });
      mast.addEventListener("focusin", () => {
        focusWithin = true;
        setState("shown");
      });
      mast.addEventListener("focusout", (e: FocusEvent) => {
        const next = e.relatedTarget as Node | null;
        if (!next || !mast.contains(next)) {
          focusWithin = false;
        }
      });
    }
  }
  syncCssVars();

  rmQuery?.addEventListener?.("change", (e) => {
    reducedMotion = e.matches;
    if (reducedMotion) setState("shown");
  });

  // Scroll choreography: passive + rAF throttled + anti-bounce hysteresis + 3000ms idle reveal
  let rafId = 0;
  let idleTimer = 0;

  const scheduleIdleReveal = (): void => {
    if (idleTimer) window.clearTimeout(idleTimer);
    idleTimer = window.setTimeout(() => {
      setState("shown");
    }, 3000);
  };

  const onScrollFrame = (): void => {
    rafId = 0;
    const y = window.scrollY;
    root.toggleAttribute("data-scrolled", y > 8);
    if (programmaticScroll) {
      lastY = y;
      acc = 0;
      return;
    }
    const dy = y - lastY;
    lastY = y;
    if (dy !== 0) {
      if ((dy > 0 && acc < 0) || (dy < 0 && acc > 0)) acc = 0;
      acc += dy;
    }
    if (y <= 120 || acc <= -24) {
      setState("shown");
    } else if (y > 120 && acc >= 32 && !isPinned()) {
      setState("hidden");
    }
  };
  root.toggleAttribute("data-scrolled", window.scrollY > 8);

  window.addEventListener(
    "scroll",
    () => {
      scheduleIdleReveal();
      if (!rafId) rafId = window.requestAnimationFrame(onScrollFrame);
    },
    { passive: true }
  );

  // Fine-pointer top-edge reveal
  window.addEventListener(
    "mousemove",
    (e: MouseEvent) => {
      if (state === "hidden" && fineQuery?.matches && e.clientY <= 12) {
        setState("shown");
      }
    },
    { passive: true }
  );

  // Theme switching: system -> light -> dark
  const themeBtns = Array.from(
    document.querySelectorAll<HTMLButtonElement>("[data-theme-toggle]")
  );

  const readTheme = (): ThemeMode => {
    try {
      const raw = localStorage.getItem(THEME_KEY);
      if (raw === "light" || raw === "dark" || raw === "system") return raw;
    } catch {
      // Ignore storage access errors.
    }
    return "system";
  };

  const themeLabel = (mode: ThemeMode): string => {
    if (isZh()) {
      if (mode === "light") return "\u4e3b\u9898\uff1a\u6d45\u8272";
      if (mode === "dark") return "\u4e3b\u9898\uff1a\u6df1\u8272";
      return "\u4e3b\u9898\uff1a\u8ddf\u968f\u7cfb\u7edf";
    }
    if (mode === "light") return "Theme: Light";
    if (mode === "dark") return "Theme: Dark";
    return "Theme: System";
  };

  const syncThemeMeta = (mode: ThemeMode): void => {
    const dk =
      mode === "dark" || (mode !== "light" && Boolean(darkQuery?.matches));
    const cs = dk ? "dark" : "light";
    root.style.colorScheme = cs;
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute("content", dk ? "#121110" : "#f6f3ec");
    const csMeta = document.querySelector('meta[name="color-scheme"]');
    if (csMeta) csMeta.setAttribute("content", cs);
  };

  const syncThemeButtons = (mode: ThemeMode): void => {
    const label = themeLabel(mode);
    for (const btn of themeBtns) {
      btn.setAttribute("aria-label", label);
      btn.setAttribute("title", label);
      btn.innerHTML = THEME_SVGS[mode];
    }
  };

  const applyTheme = (mode: ThemeMode, animateSwap: boolean): void => {
    if (animateSwap) {
      root.classList.add("theme-swap");
      window.requestAnimationFrame(() => {
        window.requestAnimationFrame(() => {
          root.classList.remove("theme-swap");
        });
      });
    }
    if (mode === "system") {
      delete root.dataset.theme;
    } else {
      root.dataset.theme = mode;
    }
    try {
      localStorage.setItem(THEME_KEY, mode);
    } catch {
      // Ignore storage errors.
    }
    syncThemeMeta(mode);
    syncThemeButtons(mode);
  };

  let curTheme = readTheme();
  applyTheme(curTheme, false);

  const cycleTheme = (): void => {
    const idx = THEME_MODES.indexOf(curTheme);
    curTheme = THEME_MODES[(idx + 1) % THEME_MODES.length];
    applyTheme(curTheme, true);
  };

  for (const btn of themeBtns) {
    btn.addEventListener("click", cycleTheme);
  }
  darkQuery?.addEventListener?.("change", () => {
    if (curTheme === "system") syncThemeMeta("system");
  });

  // Language link preserving ?q / ?tag + saving exact story-anchor lock (with #anchor fallback)
  const buildLangHref = (
    link: HTMLAnchorElement,
    fallbackHash?: string
  ): string => {
    const raw = link.getAttribute("href") || "";
    if (!raw) return raw;
    try {
      const target = new URL(raw, window.location.href);
      const cur = new URL(window.location.href);
      const q = cur.searchParams.get("q");
      const tag = cur.searchParams.get("tag");
      if (q) target.searchParams.set("q", q);
      else target.searchParams.delete("q");
      if (tag) target.searchParams.set("tag", tag);
      else target.searchParams.delete("tag");
      target.hash =
        fallbackHash !== undefined ? fallbackHash : cur.hash;
      return target.pathname + target.search + target.hash;
    } catch {
      return raw;
    }
  };

  const saveLangAnchor = (): LangAnchorSnapshot => {
    const snap = captureVisibleAnchor(document, mastH);
    try {
      sessionStorage.setItem(LANG_ANCHOR_KEY, JSON.stringify(snap));
    } catch {
      // Ignore storage restrictions.
    }
    return snap;
  };

  const langLinks = Array.from(
    document.querySelectorAll<HTMLAnchorElement>("[data-lang-link], .lang-seg a[href]")
  );
  for (const link of langLinks) {
    const update = (hashOverride?: string): void => {
      const nextHref = buildLangHref(link, hashOverride);
      if (nextHref) link.setAttribute("href", nextHref);
    };
    link.addEventListener("mouseenter", () => update());
    link.addEventListener("focus", () => update());
    link.addEventListener("click", () => {
      const snap = saveLangAnchor();
      const hashFallback =
        snap.anchorId && snap.anchorId !== "__top__"
          ? `#${snap.anchorId}`
          : window.location.hash;
      update(hashFallback);
    });
  }

  // Global shortcuts: t (theme), l (lang)
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.metaKey || e.ctrlKey || e.altKey || isTypingTarget(e.target)) return;
    if (e.key === "t" || e.key === "T") {
      e.preventDefault();
      cycleTheme();
    } else if (e.key === "l" || e.key === "L") {
      const link = langLinks[0];
      if (link) {
        e.preventDefault();
        const snap = saveLangAnchor();
        const hashFallback =
          snap.anchorId && snap.anchorId !== "__top__"
            ? `#${snap.anchorId}`
            : window.location.hash;
        const nextHref = buildLangHref(link, hashFallback);
        if (nextHref) window.location.assign(nextHref);
        return;
      }
      const targetFor = isZh() ? "en-language" : "language-zh";
      const label = document.querySelector<HTMLElement>(
        `label[for="${targetFor}"]`
      );
      if (label) {
        e.preventDefault();
        label.click();
        return;
      }
      const zhRadio = document.getElementById("language-zh") as HTMLInputElement | null;
      const enRadio = document.getElementById("en-language") as HTMLInputElement | null;
      if (zhRadio && enRadio) {
        e.preventDefault();
        const nextRadio = isZh() ? enRadio : zhRadio;
        nextRadio.checked = true;
        nextRadio.dispatchEvent(new Event("change", { bubbles: true }));
      }
    }
  });

  return {
    setPinned,
    setState,
    setProgrammaticScroll,
    getMastHeight: () => mastH,
    getMastOffset: () => (state === "shown" ? mastH : 0),
    isReducedMotion: () => reducedMotion,
    isZh,
    isTypingTarget,
    isPinned,
    syncLang: () => {
      syncThemeButtons(curTheme);
      measureMast();
    },
  };
}
