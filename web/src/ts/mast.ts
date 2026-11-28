export type ThemeMode = "system" | "light" | "dark";
export type FontSize = "sm" | "md" | "lg";

export interface MastController {
  getMastHeight(): number;
  getTopOffset(): number;
  isReducedMotion(): boolean;
  isZh(): boolean;
  isTypingTarget(el: EventTarget | null): boolean;
  syncLang(): void;
}

const THEME_KEY = "digest-theme";
const FONT_KEY = "digest-font";
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
  mastHeight = 52
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
  const sentinels = Array.from(
    document.querySelectorAll<HTMLElement>(".mast-sentinel")
  );
  const isZh = (): boolean => (root.lang || "").toLowerCase().startsWith("zh");

  const rmQuery =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-reduced-motion: reduce)")
      : null;
  const darkQuery =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;

  let reducedMotion = Boolean(rmQuery?.matches);
  let mastH = window.innerWidth <= 960 ? 48 : 52;

  const syncMastVar = (): void => {
    root.style.setProperty("--mast-h", `${mastH}px`);
  };

  const measureMast = (): void => {
    for (const mast of masts) {
      const h = Math.round(mast.getBoundingClientRect().height);
      if (h > 0) {
        if (h !== mastH) {
          mastH = h;
          syncMastVar();
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
      ro?.observe(mast);
    }
  }
  syncMastVar();

  rmQuery?.addEventListener?.("change", (e) => {
    reducedMotion = e.matches;
  });

  // Toggle .mast.is-scrolled via 4px top sentinel IntersectionObserver (no scroll listener)
  const setScrolled = (scrolled: boolean): void => {
    for (const mast of masts) {
      mast.classList.toggle("is-scrolled", scrolled);
    }
  };

  if (sentinels.length > 0 && typeof IntersectionObserver !== "undefined") {
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          setScrolled(!entry.isIntersecting);
        }
      },
      { threshold: 0 }
    );
    for (const s of sentinels) io.observe(s);
  } else {
    setScrolled(window.scrollY > 4);
  }

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

  const resolveEffectiveTheme = (mode: ThemeMode): "dark" | "light" =>
    mode === "dark" || (mode !== "light" && Boolean(darkQuery?.matches))
      ? "dark"
      : "light";

  const syncThemeMeta = (mode: ThemeMode): void => {
    const dk = resolveEffectiveTheme(mode) === "dark";
    const cs = dk ? "dark" : "light";
    root.style.colorScheme = cs;
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) {
      meta.setAttribute("content", dk ? __THEME_DARK_BG__ : __THEME_LIGHT_BG__);
    }
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

  const applyThemeDom = (mode: ThemeMode): void => {
    if (mode === "system") {
      delete root.dataset.theme;
    } else {
      root.dataset.theme = mode;
    }
    syncThemeMeta(mode);
  };

  let activeVt: {
    skipTransition?: () => void;
    finished?: Promise<void>;
  } | null = null;

  let curTheme = readTheme();
  applyThemeDom(curTheme);
  syncThemeButtons(curTheme);

  const cycleTheme = (ev?: MouseEvent): void => {
    const prevEffective = resolveEffectiveTheme(curTheme);
    const idx = THEME_MODES.indexOf(curTheme);
    const nextMode = THEME_MODES[(idx + 1) % THEME_MODES.length];
    const nextEffective = resolveEffectiveTheme(nextMode);
    curTheme = nextMode;
    try {
      localStorage.setItem(THEME_KEY, nextMode);
    } catch {
      // Ignore storage errors.
    }
    syncThemeButtons(nextMode);

    if (prevEffective === nextEffective) {
      applyThemeDom(nextMode);
      return;
    }

    const docWithVt = document as Document & {
      startViewTransition?: (cb: () => void) => {
        ready: Promise<void>;
        finished?: Promise<void>;
        skipTransition?: () => void;
      };
    };

    const hadActiveVt = activeVt !== null;
    if (hadActiveVt) {
      try {
        activeVt?.skipTransition?.();
      } catch {
        // Ignore skip errors.
      }
      activeVt = null;
      root.classList.remove("is-theme-vt");
    }

    if (
      !reducedMotion &&
      !hadActiveVt &&
      typeof docWithVt.startViewTransition === "function"
    ) {
      const trigger =
        (ev?.currentTarget instanceof HTMLElement ? ev.currentTarget : null) ??
        themeBtns.find((b) => b.offsetParent !== null) ??
        themeBtns[0] ??
        null;
      const rect = trigger?.getBoundingClientRect();
      const x =
        ev && ev.detail > 0
          ? ev.clientX
          : rect
            ? rect.left + rect.width / 2
            : Math.max(24, window.innerWidth - 56);
      const y =
        ev && ev.detail > 0
          ? ev.clientY
          : rect
            ? rect.top + rect.height / 2
            : 26;
      const endRadius = Math.hypot(
        Math.max(x, window.innerWidth - x),
        Math.max(y, window.innerHeight - y)
      );
      root.classList.add("is-theme-vt");
      const vt = docWithVt.startViewTransition(() => {
        if (curTheme !== nextMode) return;
        applyThemeDom(nextMode);
      });
      activeVt = vt;
      vt.finished
        ?.catch(() => {})
        .finally(() => {
          if (activeVt === vt) {
            activeVt = null;
            root.classList.remove("is-theme-vt");
          }
        });
      vt.ready
        .then(() => {
          root.animate(
            {
              clipPath: [
                `circle(0px at ${x}px ${y}px)`,
                `circle(${endRadius}px at ${x}px ${y}px)`,
              ],
            },
            {
              duration: __MOTION_THEME_REVEAL_MS__,
              delay: __MOTION_THEME_REVEAL_DELAY_MS__,
              easing: __MOTION_EASE__,
              fill: "both",
              pseudoElement: "::view-transition-new(root)",
            }
          );
        })
        .catch(() => {});
    } else if (!reducedMotion) {
      root.classList.add("theme-swap");
      applyThemeDom(nextMode);
      window.requestAnimationFrame(() => {
        window.requestAnimationFrame(() => {
          root.classList.remove("theme-swap");
        });
      });
    } else {
      applyThemeDom(nextMode);
    }
  };

  for (const btn of themeBtns) {
    btn.addEventListener("click", (ev) => cycleTheme(ev));
  }
  darkQuery?.addEventListener?.("change", () => {
    if (curTheme === "system") syncThemeMeta("system");
  });

  // 3-tier font size switcher: sm / md / lg (persisted, pre-paint synced via boot.ts)
  const fontBtns = Array.from(
    document.querySelectorAll<HTMLButtonElement>("[data-font-size]")
  );
  const readFont = (): FontSize => {
    try {
      const raw = localStorage.getItem(FONT_KEY);
      if (raw === "sm" || raw === "md" || raw === "lg") return raw;
    } catch {
      // Ignore storage access errors.
    }
    return "md";
  };
  const applyFont = (size: FontSize, save: boolean): void => {
    root.dataset.font = size;
    if (save) {
      try {
        localStorage.setItem(FONT_KEY, size);
      } catch {
        // Ignore storage errors.
      }
    }
    for (const btn of fontBtns) {
      btn.setAttribute("aria-pressed", btn.dataset.fontSize === size ? "true" : "false");
    }
  };
  if (fontBtns.length > 0) {
    applyFont(readFont(), false);
    for (const btn of fontBtns) {
      btn.addEventListener("click", () => {
        const next = btn.dataset.fontSize;
        if (next === "sm" || next === "md" || next === "lg") {
          applyFont(next, true);
        }
      });
    }
  }

  // SEC-9: Preserve origin for approved cross-origin LANG_ALT without forwarding
  // query/hash cross-origin, and reject protocol-relative // pathnames.
  const buildLangHref = (
    link: HTMLAnchorElement,
    fallbackHash?: string
  ): string => {
    const raw = link.getAttribute("href") || "";
    if (!raw || raw.startsWith("//")) return "";
    try {
      const target = new URL(raw, window.location.href);
      if (target.protocol !== "https:" && target.protocol !== "http:") {
        return "";
      }
      if (target.pathname.startsWith("//")) return "";
      const cur = new URL(window.location.href);
      if (target.origin !== cur.origin) {
        return target.origin + target.pathname;
      }
      for (const param of ["cat", "q", "tag"]) {
        const val = cur.searchParams.get(param);
        if (val) target.searchParams.set(param, val);
        else target.searchParams.delete(param);
      }
      target.hash = fallbackHash !== undefined ? fallbackHash : cur.hash;
      return target.pathname + target.search + target.hash;
    } catch {
      return "";
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
      }
    }
  });

  const getTopOffset = (): number => {
    if (window.innerWidth <= 960) {
      const strip = document.querySelector<HTMLElement>(".mobile-filter-strip");
      const stripH = strip ? Math.round(strip.getBoundingClientRect().height) : 0;
      return mastH + stripH;
    }
    return mastH;
  };

  return {
    getMastHeight: () => mastH,
    getTopOffset,
    isReducedMotion: () => reducedMotion,
    isZh,
    isTypingTarget,
    syncLang: () => syncThemeButtons(curTheme),
  };
}
