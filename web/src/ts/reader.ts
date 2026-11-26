import { initFilter } from "./filter";
import {
  captureVisibleAnchor,
  initMast,
  LANG_ANCHOR_KEY,
  type LangAnchorSnapshot,
  type MastController,
} from "./mast";

const TOC_ALL_KEY = "digest-toc-all";
const LONG_JUMP_PX = 900;
const SHORT_JUMP_MAX_MS = 200;

const CHECK_SVG =
  '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 8.5l3.2 3.2L13 4.8"/></svg>';

interface SpyEntry {
  id: string;
  secId: string;
  el: HTMLElement;
  top: number;
}

interface PaneController {
  pane: Document | HTMLElement;
  activate(anchorSnap?: LangAnchorSnapshot | null): void;
  deactivate(): void;
  rebuildSpyCache(): void;
  onScroll(): void;
  jumpToElement(el: HTMLElement, updateHash?: boolean): void;
}

function consumeLangAnchor(): LangAnchorSnapshot | null {
  try {
    const raw = sessionStorage.getItem(LANG_ANCHOR_KEY);
    if (!raw) return null;
    sessionStorage.removeItem(LANG_ANCHOR_KEY);
    const parsed = JSON.parse(raw) as Partial<LangAnchorSnapshot>;
    if (
      typeof parsed.anchorId === "string" &&
      typeof parsed.relTop === "number" &&
      typeof parsed.ts === "number" &&
      Date.now() - parsed.ts < 5000
    ) {
      return parsed as LangAnchorSnapshot;
    }
  } catch {
    // Ignore storage/parse errors.
  }
  return null;
}

function findAnchorInScope(
  scope: Document | HTMLElement,
  anchorId: string
): HTMLElement | null {
  if (!anchorId || anchorId === "__top__") return null;
  const escaped = CSS.escape(anchorId);
  return scope.querySelector<HTMLElement>(`#${escaped}, #en-${escaped}`);
}

function readExpandAllPref(defaultAll: boolean): boolean {
  try {
    const v = sessionStorage.getItem(TOC_ALL_KEY);
    if (v === "1") return true;
    if (v === "0") return false;
  } catch {
    // Ignore storage restrictions.
  }
  return defaultAll;
}

function initReaderPane(
  scope: Document | HTMLElement,
  mast: MastController,
  isActive: () => boolean,
  onCrossPaneTarget?: (el: HTMLElement) => void
): PaneController {
  const root = document.documentElement;
  const mainEl = scope.querySelector<HTMLElement>("main.prose-col, main");
  const mastEl = scope.querySelector<HTMLElement>("header.mast");
  const filterDrawerEl = scope.querySelector<HTMLElement>(".filter-drawer");
  const tocEl = scope.querySelector<HTMLElement>("nav.toc-side");
  const dockEl = scope.querySelector<HTMLElement>("nav.dock");
  const dockTocBtn = scope.querySelector<HTMLButtonElement>("[data-sheet-open]");
  const dockNavEl = scope.querySelector<HTMLElement>("[data-dock-nav]");
  const dockTopBtn = scope.querySelector<HTMLButtonElement>("[data-scroll-top]");
  const toTopBtn = scope.querySelector<HTMLButtonElement>("[data-to-top]");
  const scrimEl = scope.querySelector<HTMLElement>("[data-sheet-scrim]");
  const tocCloseBtn = tocEl?.querySelector<HTMLButtonElement>(".toc-close") ?? null;
  const expandAllBtn = tocEl?.querySelector<HTMLButtonElement>(".toc-expand-all") ?? null;
  const liveEl = scope.querySelector<HTMLElement>("[aria-live='polite'].sr-only, #liveAnnounce");

  const paneIsZh = (): boolean => {
    if (scope instanceof HTMLElement) {
      const lang = scope.getAttribute("lang") || scope.dataset.lang || "";
      if (lang) return lang.toLowerCase().startsWith("zh");
    }
    return mast.isZh();
  };

  const stories = Array.from(scope.querySelectorAll<HTMLElement>("article.story"));
  const sections = Array.from(scope.querySelectorAll<HTMLElement>("section.sec"));
  const tsGroups = tocEl
    ? Array.from(tocEl.querySelectorAll<HTMLElement>(".ts-group"))
    : [];

  // Mirror prev/next links from nav.issue-nav.top into .dock-nav (single source of truth)
  const topNav = scope.querySelector<HTMLElement>("nav.issue-nav.top");
  if (topNav && dockNavEl) {
    const prevEl = topNav.querySelector('a[rel="prev"], [data-nav="prev"]');
    const nextEl = topNav.querySelector('a[rel="next"], [data-nav="next"]');
    if (prevEl) dockNavEl.appendChild(prevEl.cloneNode(true));
    if (nextEl) dockNavEl.appendChild(nextEl.cloneNode(true));
  }

  // All TOC sections expanded by default (user mandate #193: Cmd+F & full scannability).
  // Manual per-section toggle and Collapse-all / Expand-all remain opt-in.
  let allExpanded = readExpandAllPref(true);
  let tocHovering = false;
  let spyEntries: SpyEntry[] = [];
  let activeId = "";
  let activeSecId = "";
  let spyLocked = false;
  let spySettleTimer = 0;
  let scrollEndHandler: (() => void) | null = null;
  let jumpRafId = 0;
  let cancelUserInterrupt: (() => void) | null = null;
  let arrivedTimer = 0;
  let lastArrivedEl: HTMLElement | null = null;

  tocEl?.addEventListener("mouseenter", () => {
    tocHovering = true;
  });
  tocEl?.addEventListener("mouseleave", () => {
    tocHovering = false;
  });

  const syncExpandAllLabel = (): void => {
    if (!expandAllBtn) return;
    const anyOpen = tsGroups.some(
      (g) => !g.hidden && g.dataset.open !== "false"
    );
    allExpanded = anyOpen;
    if (paneIsZh()) {
      expandAllBtn.textContent = anyOpen ? "\u5168\u90e8\u6536\u8d77" : "\u5168\u90e8\u5c55\u5f00";
    } else {
      expandAllBtn.textContent = anyOpen ? "Collapse all" : "Expand all";
    }
    expandAllBtn.setAttribute("aria-pressed", anyOpen ? "true" : "false");
    expandAllBtn.setAttribute("aria-expanded", anyOpen ? "true" : "false");
  };

  const setGroupOpen = (group: HTMLElement, open: boolean): void => {
    const btn = group.querySelector<HTMLButtonElement>(".ts-sec");
    const panel =
      group.querySelector<HTMLElement>(".ts-panel") ??
      group.querySelector<HTMLElement>("ol.ts-items");
    group.dataset.open = open ? "true" : "false";
    if (btn) btn.setAttribute("aria-expanded", open ? "true" : "false");
    if (panel) {
      if (panel.classList.contains("ts-panel")) {
        (panel as HTMLElement & { inert: boolean }).inert = !open;
        panel.setAttribute("aria-hidden", open ? "false" : "true");
      } else {
        panel.hidden = !open;
      }
    }
  };

  const applyAllGroups = (open: boolean): void => {
    for (const grp of tsGroups) {
      setGroupOpen(grp, !grp.hidden && open);
    }
    syncExpandAllLabel();
  };

  // All sections start expanded unless user explicitly collapsed in sessionStorage
  applyAllGroups(allExpanded);

  expandAllBtn?.addEventListener("click", () => {
    const next = !allExpanded;
    try {
      sessionStorage.setItem(TOC_ALL_KEY, next ? "1" : "0");
    } catch {
      // Ignore storage restrictions.
    }
    applyAllGroups(next);
  });

  // Individual accordion section toggle (purely manual opt-in)
  tocEl?.addEventListener("click", (e: MouseEvent) => {
    const secBtn = (e.target as Element | null)?.closest<HTMLButtonElement>(".ts-sec");
    if (!secBtn) return;
    const grp = secBtn.closest<HTMLElement>(".ts-group");
    if (!grp) return;
    const isOpen = secBtn.getAttribute("aria-expanded") === "true";
    setGroupOpen(grp, !isOpen);
    syncExpandAllLabel();
  });

  // Mobile sheet open / close + inert + focus trap
  let sheetOpen = false;
  const setInert = (val: boolean): void => {
    for (const el of [mainEl, mastEl, filterDrawerEl, dockEl]) {
      if (!el) continue;
      if (el === filterDrawerEl && !val && !el.classList.contains("is-open")) {
        continue;
      }
      (el as HTMLElement & { inert: boolean }).inert = val;
    }
  };

  const closeSheet = (restoreFocus = true): void => {
    if (!sheetOpen || !tocEl) return;
    sheetOpen = false;
    tocEl.classList.remove("is-open");
    if (scrimEl) {
      scrimEl.classList.remove("is-open");
      scrimEl.hidden = true;
    }
    root.classList.remove("is-sheet-open");
    setInert(false);
    mast.setPinned("sheet", false);
    dockTocBtn?.setAttribute("aria-expanded", "false");
    if (restoreFocus) dockTocBtn?.focus();
  };

  const openSheet = (): void => {
    if (sheetOpen || !tocEl || !isActive()) return;
    sheetOpen = true;
    if (scrimEl) {
      scrimEl.hidden = false;
      void scrimEl.offsetWidth;
      scrimEl.classList.add("is-open");
    }
    tocEl.classList.add("is-open");
    root.classList.add("is-sheet-open");
    setInert(true);
    mast.setPinned("sheet", true);
    dockTocBtn?.setAttribute("aria-expanded", "true");
    const curLink = tocEl.querySelector<HTMLElement>("a.cur");
    const target =
      (curLink && !curLink.closest("[inert]") ? curLink : null) ||
      tocEl.querySelector<HTMLElement>(".ts-sec.cur") ||
      tocEl.querySelector<HTMLElement>(".ts-sec") ||
      tocCloseBtn;
    target?.focus();
  };

  dockTocBtn?.addEventListener("click", openSheet);
  tocCloseBtn?.addEventListener("click", () => closeSheet(true));
  scrimEl?.addEventListener("click", () => closeSheet(true));

  // Focus trap inside open sheet + Esc dismiss
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (!sheetOpen || !tocEl || !isActive()) return;
    if (e.key === "Escape") {
      e.preventDefault();
      closeSheet(true);
      return;
    }
    if (e.key === "Tab") {
      const focusables = Array.from(
        tocEl.querySelectorAll<HTMLElement>(
          'a[href]:not([hidden]), button:not([disabled]):not([hidden]), [tabindex]:not([tabindex="-1"])'
        )
      ).filter((el) => el.offsetParent !== null && !el.closest("[inert]"));
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  });

  // Clean spy-lock and programmatic scroll helpers
  const clearScrollEndHook = (): void => {
    if (spySettleTimer) {
      window.clearTimeout(spySettleTimer);
      spySettleTimer = 0;
    }
    if (scrollEndHandler) {
      window.removeEventListener("scrollend", scrollEndHandler);
      scrollEndHandler = null;
    }
  };

  const releaseSpyLock = (): void => {
    clearScrollEndHook();
    spyLocked = false;
    mast.setProgrammaticScroll(false);
  };

  const cancelJumpAnim = (): void => {
    if (jumpRafId) {
      window.cancelAnimationFrame(jumpRafId);
      jumpRafId = 0;
    }
    if (cancelUserInterrupt) {
      cancelUserInterrupt();
      cancelUserInterrupt = null;
    }
    clearScrollEndHook();
  };

  const armSpySettle = (delayMs: number): void => {
    clearScrollEndHook();
    const finish = (): void => {
      releaseSpyLock();
    };
    scrollEndHandler = finish;
    window.addEventListener("scrollend", finish, { once: true });
    spySettleTimer = window.setTimeout(finish, delayMs);
  };

  const triggerArrived = (el: HTMLElement): void => {
    if (mast.isReducedMotion()) return;
    if (arrivedTimer) {
      window.clearTimeout(arrivedTimer);
      arrivedTimer = 0;
    }
    if (lastArrivedEl) {
      lastArrivedEl.classList.remove("is-arrived");
      lastArrivedEl = null;
    }
    void el.offsetWidth;
    el.classList.add("is-arrived");
    lastArrivedEl = el;
    arrivedTimer = window.setTimeout(() => {
      el.classList.remove("is-arrived");
      if (lastArrivedEl === el) lastArrivedEl = null;
      arrivedTimer = 0;
    }, 1150);
  };

  // Distance-adaptive fast scroll:
  //   <= 900px -> custom rAF cubic-ease-out capped strictly at <= 200ms
  //   >  900px (or reduced motion) -> 0ms instant jump
  const fastScrollTo = (
    destY: number,
    onLanded?: (wasLongJump: boolean) => void
  ): void => {
    cancelJumpAnim();
    const targetY = Math.max(0, Math.round(destY));
    const startY = window.scrollY;
    const dist = Math.abs(targetY - startY);

    spyLocked = true;
    mast.setProgrammaticScroll(true);

    if (mast.isReducedMotion() || dist > LONG_JUMP_PX || dist < 2) {
      window.scrollTo({
        top: targetY,
        behavior: "instant" as ScrollBehavior,
      });
      onLanded?.(dist > LONG_JUMP_PX && !mast.isReducedMotion());
      armSpySettle(120);
      return;
    }

    const duration = Math.min(
      SHORT_JUMP_MAX_MS,
      Math.max(90, Math.round(dist * 0.22))
    );
    const t0 = performance.now();

    const onInterrupt = (): void => {
      cancelJumpAnim();
      releaseSpyLock();
    };
    window.addEventListener("wheel", onInterrupt, { once: true, passive: true });
    window.addEventListener("touchstart", onInterrupt, { once: true, passive: true });
    window.addEventListener("pointerdown", onInterrupt, { once: true, passive: true });
    cancelUserInterrupt = () => {
      window.removeEventListener("wheel", onInterrupt);
      window.removeEventListener("touchstart", onInterrupt);
      window.removeEventListener("pointerdown", onInterrupt);
    };

    const step = (now: number): void => {
      const p = Math.min(1, (now - t0) / duration);
      const ease = 1 - (1 - p) * (1 - p) * (1 - p);
      window.scrollTo(0, Math.round(startY + (targetY - startY) * ease));
      if (p < 1) {
        jumpRafId = window.requestAnimationFrame(step);
      } else {
        jumpRafId = 0;
        if (cancelUserInterrupt) {
          cancelUserInterrupt();
          cancelUserInterrupt = null;
        }
        window.scrollTo(0, targetY);
        onLanded?.(false);
        armSpySettle(100);
      }
    };
    jumpRafId = window.requestAnimationFrame(step);
  };

  // Scroll-to-top buttons (mobile dock + desktop floating pill)
  const scrollToTop = (): void => {
    mast.setState("shown");
    fastScrollTo(0);
  };
  dockTopBtn?.addEventListener("click", scrollToTop);
  toTopBtn?.addEventListener("click", scrollToTop);

  // Scroll-spy offset cache + binary search (scoped to active pane)
  const rebuildSpyCache = (): void => {
    if (!isActive()) {
      spyEntries = [];
      return;
    }
    const next: SpyEntry[] = [];
    for (const sec of sections) {
      if (sec.hidden) continue;
      const secId = sec.id;
      const secStories = Array.from(
        sec.querySelectorAll<HTMLElement>("article.story")
      ).filter((s) => !s.hidden && s.id);
      if (secStories.length === 0 && secId) {
        next.push({
          id: secId,
          secId,
          el: sec,
          top: sec.getBoundingClientRect().top + window.scrollY,
        });
      } else {
        for (const st of secStories) {
          next.push({
            id: st.id,
            secId,
            el: st,
            top: st.getBoundingClientRect().top + window.scrollY,
          });
        }
      }
    }
    next.sort((a, b) => a.top - b.top);
    spyEntries = next;
  };

  const keepTocRowVisible = (link: HTMLElement): void => {
    if (!tocEl || sheetOpen || tocHovering) return;
    if (window.innerWidth < 1024) return;
    if (link.closest("[inert]")) return;
    const h = tocEl.clientHeight;
    if (h <= 0) return;
    const top = link.offsetTop;
    const bottom = top + link.offsetHeight;
    const cur = tocEl.scrollTop;
    const pad = 28;
    if (top < cur + pad) {
      tocEl.scrollTop = Math.max(0, top - pad);
    } else if (bottom > cur + h - pad) {
      tocEl.scrollTop = Math.max(0, bottom - h + pad);
    }
  };

  const setActiveEntry = (
    entry: SpyEntry | null,
    expandIfClosed = false
  ): void => {
    if (!entry) return;
    if (entry.id === activeId && entry.secId === activeSecId) return;
    activeId = entry.id;
    const secChanged = entry.secId !== activeSecId;
    activeSecId = entry.secId;

    if (tocEl) {
      tocEl.querySelector<HTMLElement>("a.cur")?.classList.remove("cur");
      const nextLink = tocEl.querySelector<HTMLElement>(
        `a[href="#${CSS.escape(entry.id)}"]`
      );
      if (nextLink) nextLink.classList.add("cur");

      if (secChanged) {
        tocEl.querySelector<HTMLElement>(".ts-sec.cur")?.classList.remove("cur");
        const grp = activeSecId
          ? tocEl.querySelector<HTMLElement>(
              `.ts-group[data-sec="${CSS.escape(activeSecId)}"]`
            )
          : null;
        grp?.querySelector<HTMLElement>(".ts-sec")?.classList.add("cur");
        // NEVER auto-collapse any section; only open the clicked section if user had collapsed it
        if (expandIfClosed && grp && grp.dataset.open === "false") {
          setGroupOpen(grp, true);
          syncExpandAllLabel();
        }
      }
      if (nextLink) keepTocRowVisible(nextLink);
    }

    // Update mobile dock label with active section title
    if (dockTocBtn && activeSecId) {
      const grp = tocEl?.querySelector<HTMLElement>(
        `.ts-group[data-sec="${CSS.escape(activeSecId)}"] .ts-title`
      );
      const secHeading = scope.querySelector<HTMLElement>(
        `#${CSS.escape(activeSecId)} .sec-head h2`
      );
      const title = grp?.textContent?.trim() || secHeading?.textContent?.trim() || "";
      const labelSpan = dockTocBtn.querySelector<HTMLElement>("span");
      if (labelSpan && title) {
        const prefix = paneIsZh() ? "\u76ee\u5f55 \u00b7 " : "Contents \u00b7 ";
        labelSpan.textContent = prefix + title;
      }
    }
  };

  const updateSpy = (): void => {
    if (!isActive() || spyLocked) return;
    if (spyEntries.length === 0) return;
    const filterH =
      filterDrawerEl && filterDrawerEl.classList.contains("is-open")
        ? Math.round(filterDrawerEl.getBoundingClientRect().height)
        : 0;
    const probeY = window.scrollY + mast.getMastOffset() + filterH + 32;
    let lo = 0;
    let hi = spyEntries.length - 1;
    let best = 0;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (spyEntries[mid].top <= probeY) {
        best = mid;
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    setActiveEntry(spyEntries[best], false);
  };

  const onScroll = (): void => {
    if (!isActive()) return;
    const docH = document.documentElement.scrollHeight - window.innerHeight;
    const prog = docH > 0 ? Math.min(1, Math.max(0, window.scrollY / docH)) : 0;
    root.style.setProperty("--read-prog", prog.toFixed(4));
    if (dockTopBtn) dockTopBtn.hidden = window.scrollY <= 600;
    if (toTopBtn) {
      const show = window.scrollY > 420;
      toTopBtn.classList.toggle("is-show", show);
      (toTopBtn as HTMLButtonElement & { inert: boolean }).inert = !show;
      toTopBtn.setAttribute("aria-hidden", show ? "false" : "true");
    }
    updateSpy();
  };

  // Distance-adaptive TOC / Must-Read / Related jump with full-flight spy lock + arrival pulse
  const jumpToElement = (el: HTMLElement, updateHash = true): void => {
    const doScroll = (): void => {
      const targetY = el.getBoundingClientRect().top + window.scrollY;
      const goingDown = targetY > window.scrollY && !mast.isPinned();
      const filterH =
        filterDrawerEl && filterDrawerEl.classList.contains("is-open")
          ? Math.round(filterDrawerEl.getBoundingClientRect().height)
          : 0;
      const visibleHeaderH =
        (mastEl
          ? Math.round(mastEl.getBoundingClientRect().height)
          : mast.getMastHeight()) + filterH;
      const dest = goingDown
        ? targetY - 16
        : targetY - visibleHeaderH - 16;
      mast.setState(goingDown ? "hidden" : "shown");
      if (updateHash && el.id) {
        try {
          window.history.replaceState(null, "", `#${el.id}`);
        } catch {
          // Ignore history restrictions.
        }
      }
      const sec = el.closest<HTMLElement>("section.sec");
      if (el.id) {
        setActiveEntry(
          {
            id: el.id,
            secId: sec?.id || el.id,
            el,
            top: targetY,
          },
          true
        );
      }
      fastScrollTo(dest, (wasLongJump) => {
        if (wasLongJump) triggerArrived(el);
      });
    };

    if (sheetOpen) {
      closeSheet(false);
      window.requestAnimationFrame(doScroll);
    } else {
      doScroll();
    }
  };

  // Intercept in-page anchor clicks scoped to this pane
  const clickScope: EventTarget = scope instanceof Element ? scope : document;
  clickScope.addEventListener("click", (e: Event) => {
    if (!isActive()) return;
    const link = (e.target as Element | null)?.closest<HTMLAnchorElement>(
      'a[href^="#"]'
    );
    if (!link) return;
    if (link.classList.contains("skip-link")) return;
    const hash = link.getAttribute("href") || "";
    if (hash.length < 2) return;
    const id = hash.slice(1);
    const target = document.getElementById(id);
    if (!target) return;
    e.preventDefault();
    if (scope instanceof HTMLElement && !scope.contains(target) && onCrossPaneTarget) {
      onCrossPaneTarget(target);
      return;
    }
    jumpToElement(target, true);
  });

  // Copy anchor link button + live-region announcement
  const announce = (msg: string): void => {
    if (!liveEl) return;
    liveEl.textContent = "";
    window.requestAnimationFrame(() => {
      liveEl.textContent = msg;
    });
  };

  clickScope.addEventListener("click", (e: Event) => {
    if (!isActive()) return;
    const btn = (e.target as Element | null)?.closest<HTMLButtonElement>(
      ".copy-link[data-copy-anchor]"
    );
    if (!btn) return;
    e.preventDefault();
    const anchor = btn.dataset.copyAnchor || "";
    if (!anchor) return;
    const url = `${window.location.origin}${window.location.pathname}#${anchor}`;
    const finish = (): void => {
      const origHtml = btn.dataset.origSvg || btn.innerHTML;
      if (!btn.dataset.origSvg) btn.dataset.origSvg = origHtml;
      btn.classList.add("is-copied");
      btn.innerHTML = CHECK_SVG;
      announce(paneIsZh() ? "\u5df2\u590d\u5236\u94fe\u63a5" : "Link copied");
      window.setTimeout(() => {
        btn.classList.remove("is-copied");
        btn.innerHTML = origHtml;
      }, 1600);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(url).then(finish, finish);
    } else {
      finish();
    }
  });

  // Filter integration (stories + sections + TOC mirror)
  const normSearch = (s: string): string => s.replace(/\u2011/g, "-").toLowerCase();
  const storySearchCache = new Map<HTMLElement, { text: string; tags: Set<string> }>();
  for (const st of stories) {
    const rawTags = (st.dataset.tags || "")
      .split("|")
      .map((t) => t.trim())
      .filter(Boolean);
    storySearchCache.set(st, {
      text: normSearch(`${st.textContent || ""} ${rawTags.join(" ")}`),
      tags: new Set(rawTags),
    });
  }

  const filter = initFilter({
    mast,
    root: scope,
    isActive,
    onResize: () => {
      if (isActive()) {
        rebuildSpyCache();
        updateSpy();
      }
    },
    onFilter: (query, activeTags) => {
      const qLow = normSearch(query);
      const hasFilter = qLow.length > 0 || activeTags.size > 0;
      let matched = 0;

      for (const st of stories) {
        const meta = storySearchCache.get(st);
        let ok = true;
        if (meta) {
          for (const t of activeTags) {
            if (!meta.tags.has(t)) {
              ok = false;
              break;
            }
          }
          if (ok && qLow && !meta.text.includes(qLow)) ok = false;
        }
        st.hidden = !ok;
        if (ok) matched++;

        if (tocEl && st.id) {
          const tocLink = tocEl.querySelector<HTMLElement>(
            `a[href="#${CSS.escape(st.id)}"]`
          );
          const li = tocLink?.closest("li");
          if (li) li.hidden = !ok;
        }
      }

      for (const sec of sections) {
        const secStories = Array.from(
          sec.querySelectorAll<HTMLElement>("article.story")
        );
        const visCount = secStories.filter((s) => !s.hidden).length;
        sec.hidden = visCount === 0;
        const grp = tocEl?.querySelector<HTMLElement>(
          `.ts-group[data-sec="${CSS.escape(sec.id)}"]`
        );
        if (grp) {
          grp.hidden = visCount === 0;
          const cnt = grp.querySelector<HTMLElement>(".ts-cnt");
          if (cnt) {
            const total = Number(cnt.dataset.total || String(secStories.length));
            cnt.textContent = hasFilter ? `${visCount}/${total}` : `${total}`;
          }
        }
      }

      rebuildSpyCache();
      updateSpy();
      return { matched, total: stories.length };
    },
    onFirstMatchScroll: () => {
      const first = stories.find((s) => !s.hidden);
      if (first) jumpToElement(first, false);
    },
  });

  // Keyboard shortcuts: j/k (next/prev story), [/] (prev/next issue)
  let kbdIdx = -1;
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (!isActive()) return;
    if (e.metaKey || e.ctrlKey || e.altKey || mast.isTypingTarget(e.target)) return;
    if (sheetOpen) return;

    if (e.key === "j" || e.key === "k") {
      const visible = stories.filter((s) => !s.hidden);
      if (visible.length === 0) return;
      e.preventDefault();
      if (kbdIdx >= 0 && kbdIdx < visible.length) {
        visible[kbdIdx].classList.remove("kbd-focus");
      } else {
        for (const s of stories) s.classList.remove("kbd-focus");
      }
      if (e.key === "j") {
        kbdIdx = Math.min(visible.length - 1, kbdIdx + 1);
      } else {
        kbdIdx = kbdIdx <= 0 ? 0 : kbdIdx - 1;
      }
      const cur = visible[kbdIdx];
      cur.classList.add("kbd-focus");
      jumpToElement(cur, false);
      return;
    }

    if (e.key === "[" || e.key === "]") {
      const rel = e.key === "[" ? "prev" : "next";
      const navLink = scope.querySelector<HTMLAnchorElement>(
        `nav.issue-nav.top a[rel="${rel}"], nav.issue-nav.top a[data-nav="${rel}"]`
      );
      if (navLink?.href) {
        e.preventDefault();
        window.location.assign(navLink.href);
      }
    }
  });

  if (mainEl && typeof ResizeObserver !== "undefined") {
    const ro = new ResizeObserver(() => {
      if (isActive()) rebuildSpyCache();
    });
    ro.observe(mainEl);
  }

  return {
    pane: scope,
    activate(anchorSnap?: LangAnchorSnapshot | null) {
      allExpanded = readExpandAllPref(true);
      activeId = "";
      activeSecId = "";
      kbdIdx = -1;
      applyAllGroups(allExpanded);
      filter.refreshTags();
      rebuildSpyCache();

      const snap = anchorSnap ?? consumeLangAnchor();
      if (snap) {
        spyLocked = true;
        mast.setProgrammaticScroll(true);
        if (snap.anchorId === "__top__") {
          window.scrollTo({ top: 0, behavior: "instant" as ScrollBehavior });
        } else {
          const el = findAnchorInScope(scope, snap.anchorId);
          if (el) {
            const targetY = el.getBoundingClientRect().top + window.scrollY;
            const dest = Math.max(0, Math.round(targetY - snap.relTop));
            window.scrollTo({ top: dest, behavior: "instant" as ScrollBehavior });
            const sec = el.closest<HTMLElement>("section.sec");
            setActiveEntry(
              {
                id: el.id,
                secId: sec?.id || el.id,
                el,
                top: targetY,
              },
              true
            );
          }
        }
        armSpySettle(300);
      }

      onScroll();
    },
    deactivate() {
      cancelJumpAnim();
      releaseSpyLock();
      if (arrivedTimer) {
        window.clearTimeout(arrivedTimer);
        arrivedTimer = 0;
      }
      if (lastArrivedEl) {
        lastArrivedEl.classList.remove("is-arrived");
        lastArrivedEl = null;
      }
      closeSheet(false);
      if (filter.isOpen()) filter.close(false);
      spyEntries = [];
    },
    rebuildSpyCache,
    onScroll,
    jumpToElement,
  };
}

function initReader(): void {
  const mast = initMast();
  const panes = Array.from(document.querySelectorAll<HTMLElement>(".lang-pane"));

  if (panes.length === 0) {
    // Single-language page: identical behavior scoped to document
    const ctrl = initReaderPane(document, mast, () => true);
    let scrollRaf = 0;
    window.addEventListener(
      "scroll",
      () => {
        if (!scrollRaf) {
          scrollRaf = window.requestAnimationFrame(() => {
            scrollRaf = 0;
            ctrl.onScroll();
          });
        }
      },
      { passive: true }
    );
    window.addEventListener("resize", () => ctrl.rebuildSpyCache(), { passive: true });
    document.fonts?.ready?.then(() => ctrl.rebuildSpyCache());
    ctrl.activate();
    return;
  }

  // Single-page bilingual mode (--md-en): scope TOC, filter, search & scroll-spy to active .lang-pane
  const zhRadio = document.getElementById("language-zh") as HTMLInputElement | null;
  const enRadio = document.getElementById("en-language") as HTMLInputElement | null;
  const paneControllers = new Map<HTMLElement, PaneController>();

  const paneForLang = (code: string): HTMLElement | undefined =>
    panes.find(
      (p) =>
        p.dataset.lang === code ||
        p.classList.contains(`lang-${code}`) ||
        (p.getAttribute("lang") || "").toLowerCase().startsWith(code)
    );

  const detectInitialPane = (): HTMLElement => {
    if (enRadio?.checked) return paneForLang("en") || panes[0];
    if (zhRadio?.checked) return paneForLang("zh") || panes[0];
    const hash = window.location.hash;
    if (hash && hash.length > 1) {
      const target = document.getElementById(hash.slice(1));
      const owner = target?.closest<HTMLElement>(".lang-pane");
      if (owner && panes.includes(owner)) return owner;
    }
    const visible = panes.find(
      (p) => !p.hidden && window.getComputedStyle(p).display !== "none"
    );
    return visible || panes[0];
  };

  let activePane = detectInitialPane();
  let syncingPanes = false;

  const syncDomForActivePane = (pane: HTMLElement): void => {
    syncingPanes = true;
    for (const p of panes) {
      p.hidden = p !== pane;
    }
    const isEn =
      pane.dataset.lang === "en" ||
      pane.classList.contains("lang-en") ||
      (pane.getAttribute("lang") || "").toLowerCase().startsWith("en");
    if (zhRadio) zhRadio.checked = !isEn;
    if (enRadio) enRadio.checked = isEn;
    const langAttr = pane.getAttribute("lang") || (isEn ? "en" : "zh-CN");
    document.documentElement.lang = langAttr;
    if (pane.dataset.title) document.title = pane.dataset.title;
    syncingPanes = false;
  };

  const switchToPane = (nextPane: HTMLElement, targetEl?: HTMLElement): void => {
    if (nextPane !== activePane) {
      const snap = targetEl
        ? null
        : captureVisibleAnchor(activePane, mast.getMastHeight());
      paneControllers.get(activePane)?.deactivate();
      activePane = nextPane;
      syncDomForActivePane(activePane);
      mast.syncLang();
      paneControllers.get(activePane)?.activate(snap);
    }
    if (targetEl) {
      paneControllers.get(activePane)?.jumpToElement(targetEl, true);
    }
  };

  syncDomForActivePane(activePane);
  mast.syncLang();

  for (const pane of panes) {
    const ctrl = initReaderPane(
      pane,
      mast,
      () => pane === activePane,
      (target) => {
        const owner = target.closest<HTMLElement>(".lang-pane");
        if (owner && panes.includes(owner)) {
          switchToPane(owner, target);
        }
      }
    );
    paneControllers.set(pane, ctrl);
  }

  const onRadioChange = (): void => {
    if (syncingPanes) return;
    if (enRadio?.checked) {
      const enPane = paneForLang("en");
      if (enPane) switchToPane(enPane);
    } else if (zhRadio?.checked) {
      const zhPane = paneForLang("zh");
      if (zhPane) switchToPane(zhPane);
    }
  };

  zhRadio?.addEventListener("change", onRadioChange);
  enRadio?.addEventListener("change", onRadioChange);

  // Intercept bilingual <label for="language-zh|en-language"> clicks so browser
  // never focus-scrolls to the radio input; switch panes + restore anchor synchronously.
  document.addEventListener("click", (e: MouseEvent) => {
    const label = (e.target as Element | null)?.closest<HTMLLabelElement>(
      'label[for="language-zh"], label[for="en-language"]'
    );
    if (!label) return;
    const nextPane =
      label.htmlFor === "en-language" ? paneForLang("en") : paneForLang("zh");
    if (!nextPane) return;
    e.preventDefault();
    switchToPane(nextPane);
  });

  window.addEventListener("hashchange", () => {
    const hash = window.location.hash;
    if (!hash || hash.length < 2) return;
    const target = document.getElementById(hash.slice(1));
    const owner = target?.closest<HTMLElement>(".lang-pane");
    if (owner && panes.includes(owner) && owner !== activePane) {
      switchToPane(owner, target || undefined);
    }
  });

  if (typeof MutationObserver !== "undefined") {
    const mo = new MutationObserver(() => {
      if (syncingPanes) return;
      const visible = panes.find((p) => !p.hidden);
      if (visible && visible !== activePane) {
        switchToPane(visible);
      }
    });
    for (const p of panes) {
      mo.observe(p, { attributes: true, attributeFilter: ["hidden"] });
    }
  }

  let scrollRaf = 0;
  window.addEventListener(
    "scroll",
    () => {
      if (!scrollRaf) {
        scrollRaf = window.requestAnimationFrame(() => {
          scrollRaf = 0;
          paneControllers.get(activePane)?.onScroll();
        });
      }
    },
    { passive: true }
  );

  window.addEventListener(
    "resize",
    () => paneControllers.get(activePane)?.rebuildSpyCache(),
    { passive: true }
  );
  document.fonts?.ready?.then(() =>
    paneControllers.get(activePane)?.rebuildSpyCache()
  );

  paneControllers.get(activePane)?.activate();
}

initReader();
