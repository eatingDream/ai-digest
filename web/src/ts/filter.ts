import type { MastController } from "./mast";

export interface FilterResult {
  matched: number;
  total: number;
}

export interface FilterOptions {
  mast: MastController;
  onFilter: (query: string, activeTags: Set<string>) => FilterResult;
  onFirstMatchScroll?: () => void;
  onResize?: () => void;
  root?: ParentNode;
  isActive?: () => boolean;
}

export interface FilterController {
  open(focusInput?: boolean): void;
  close(restoreFocus?: boolean): void;
  clear(): void;
  activateTag(tag: string, scrollToMatch?: boolean): void;
  isOpen(): boolean;
  refreshTags(): void;
}

const INITIAL_TAG_CAP = 12;

export function initFilter(opts: FilterOptions): FilterController {
  const { mast, onFilter, onFirstMatchScroll } = opts;
  const scope: ParentNode = opts.root ?? document;
  const isActive = opts.isActive ?? (() => true);
  const toggleBtn = scope.querySelector<HTMLButtonElement>("[data-filter-toggle]");
  const drawer =
    scope.querySelector<HTMLElement>(".filter-drawer") ??
    scope.querySelector<HTMLElement>("#digestFilterBar");
  const inner = drawer?.querySelector<HTMLElement>(".filter-inner") ?? null;
  const input = drawer?.querySelector<HTMLInputElement>(".filter-input") ?? null;
  const countEl = drawer?.querySelector<HTMLElement>(".filter-count") ?? null;
  const clearBtns = Array.from(
    scope.querySelectorAll<HTMLButtonElement>(".filter-clear")
  );
  const closeBtn = drawer?.querySelector<HTMLButtonElement>(".filter-close") ?? null;
  const tagsWrap = drawer?.querySelector<HTMLElement>(".filter-tags") ?? null;
  const emptyEl = scope.querySelector<HTMLElement>(".filter-empty");

  const activeTags = new Set<string>();
  let query = "";
  let expandedTags = false;
  let debounceTimer = 0;

  const isDrawerOpen = (): boolean =>
    Boolean(drawer && !drawer.hidden && drawer.classList.contains("is-open"));

  const syncFilterHeight = (): void => {
    if (!isActive()) return;
    const rawH =
      isDrawerOpen() && drawer
        ? Math.max(
            Math.round(drawer.getBoundingClientRect().height),
            inner?.scrollHeight ?? 0
          )
        : 0;
    const maxH = Math.max(0, window.innerHeight - mast.getMastHeight());
    const h = Math.min(rawH, maxH);
    document.documentElement.style.setProperty("--filter-h", `${h}px`);
    opts.onResize?.();
  };

  if (drawer && typeof ResizeObserver !== "undefined") {
    const ro = new ResizeObserver(syncFilterHeight);
    ro.observe(drawer);
  }

  // Ensure collapsed initial state syncs inert + aria-hidden without [hidden] snap
  if (drawer && !drawer.classList.contains("is-open")) {
    (drawer as HTMLElement & { inert: boolean }).inert = true;
    drawer.setAttribute("aria-hidden", "true");
  }

  // Snapshot original tag buttons in count-desc order
  const tagButtons: HTMLButtonElement[] = tagsWrap
    ? Array.from(tagsWrap.querySelectorAll<HTMLButtonElement>(".filter-tag:not(.filter-more)"))
    : [];
  tagButtons.forEach((btn, idx) => {
    btn.dataset.origIdx = String(idx);
  });

  // Dynamic "+N more" / "Less" toggle pill
  let moreBtn = tagsWrap?.querySelector<HTMLButtonElement>(".filter-more") ?? null;
  if (tagsWrap && !moreBtn && tagButtons.length > INITIAL_TAG_CAP) {
    moreBtn = document.createElement("button");
    moreBtn.type = "button";
    moreBtn.className = "filter-tag filter-more";
    moreBtn.hidden = true;
    tagsWrap.appendChild(moreBtn);
  }

  const labelMore = (n: number): string =>
    mast.isZh() ? `+${n} \u66f4\u591a` : `+${n} more`;
  const labelLess = (): string =>
    mast.isZh() ? "\u6536\u8d77" : "Less";

  const reorderTags = (): void => {
    if (!tagsWrap || tagButtons.length === 0) return;
    const sorted = [...tagButtons].sort((a, b) => {
      const aAct = activeTags.has(a.dataset.tag || "") ? 0 : 1;
      const bAct = activeTags.has(b.dataset.tag || "") ? 0 : 1;
      if (aAct !== bAct) return aAct - bAct;
      return Number(a.dataset.origIdx || "0") - Number(b.dataset.origIdx || "0");
    });
    for (const btn of sorted) {
      const tag = btn.dataset.tag || "";
      const active = activeTags.has(tag);
      btn.classList.toggle("is-active", active);
      btn.setAttribute("aria-pressed", active ? "true" : "false");
      tagsWrap.appendChild(btn);
    }
    if (moreBtn) tagsWrap.appendChild(moreBtn);
  };

  // Cap initial visible tags cleanly to 12 (1 clean row on desktop) with smooth +N toggle
  const layoutTags = (): void => {
    if (!tagsWrap || tagButtons.length === 0) return;
    const ordered = Array.from(
      tagsWrap.querySelectorAll<HTMLButtonElement>(".filter-tag:not(.filter-more)")
    );
    if (!moreBtn || ordered.length <= INITIAL_TAG_CAP) {
      for (const btn of ordered) btn.hidden = false;
      if (moreBtn) moreBtn.hidden = true;
      tagsWrap.classList.remove("is-expanded");
      return;
    }

    if (expandedTags) {
      for (const btn of ordered) btn.hidden = false;
      tagsWrap.classList.add("is-expanded");
      moreBtn.hidden = false;
      moreBtn.textContent = labelLess();
      moreBtn.setAttribute("aria-expanded", "true");
      tagsWrap.appendChild(moreBtn);
      return;
    }

    tagsWrap.classList.remove("is-expanded");
    let hiddenCount = 0;
    for (let i = 0; i < ordered.length; i++) {
      const show = i < INITIAL_TAG_CAP || activeTags.has(ordered[i].dataset.tag || "");
      ordered[i].hidden = !show;
      if (!show) hiddenCount++;
    }
    if (hiddenCount > 0) {
      moreBtn.hidden = false;
      moreBtn.textContent = labelMore(hiddenCount);
      moreBtn.setAttribute("aria-expanded", "false");
      tagsWrap.appendChild(moreBtn);
    } else {
      moreBtn.hidden = true;
    }
  };

  moreBtn?.addEventListener("click", () => {
    expandedTags = !expandedTags;
    layoutTags();
  });

  const syncUrl = (): void => {
    if (!isActive()) return;
    try {
      const u = new URL(window.location.href);
      if (query) u.searchParams.set("q", query);
      else u.searchParams.delete("q");
      if (activeTags.size > 0) {
        u.searchParams.set("tag", Array.from(activeTags).join(","));
      } else {
        u.searchParams.delete("tag");
      }
      window.history.replaceState(null, "", u.pathname + u.search + u.hash);
    } catch {
      // Ignore history restrictions.
    }
  };

  const apply = (updateUrl: boolean): void => {
    const hasActive = query.length > 0 || activeTags.size > 0;
    const { matched, total } = onFilter(query, activeTags);
    if (countEl) {
      countEl.textContent = hasActive ? `${matched} / ${total}` : `${total}`;
    }
    for (const btn of clearBtns) {
      btn.hidden = !hasActive;
    }
    if (emptyEl) {
      emptyEl.hidden = matched > 0 || !hasActive;
    }
    reorderTags();
    layoutTags();
    if (updateUrl) syncUrl();
  };

  const open = (focusInput = true): void => {
    if (!drawer) return;
    drawer.hidden = false;
    (drawer as HTMLElement & { inert: boolean }).inert = false;
    drawer.setAttribute("aria-hidden", "false");
    drawer.classList.add("is-open");
    toggleBtn?.setAttribute("aria-expanded", "true");
    mast.setPinned("filter", true);
    layoutTags();
    syncFilterHeight();
    if (focusInput && input) {
      input.focus();
      input.select();
    }
  };

  const close = (restoreFocus = true): void => {
    if (!drawer) return;
    drawer.classList.remove("is-open");
    (drawer as HTMLElement & { inert: boolean }).inert = true;
    drawer.setAttribute("aria-hidden", "true");
    toggleBtn?.setAttribute("aria-expanded", "false");
    mast.setPinned("filter", false);
    syncFilterHeight();
    if (restoreFocus) toggleBtn?.focus();
  };

  const clear = (): void => {
    query = "";
    activeTags.clear();
    if (input) input.value = "";
    apply(true);
  };

  const activateTag = (tag: string, scrollToMatch = false): void => {
    if (!tag) return;
    activeTags.add(tag);
    open(false);
    apply(true);
    if (scrollToMatch) onFirstMatchScroll?.();
  };

  toggleBtn?.addEventListener("click", () => {
    if (!isDrawerOpen()) open(true);
    else close(true);
  });

  closeBtn?.addEventListener("click", () => close(true));

  for (const btn of clearBtns) {
    btn.addEventListener("click", () => {
      clear();
      if (isDrawerOpen()) input?.focus();
    });
  }

  input?.addEventListener("input", () => {
    if (debounceTimer) window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(() => {
      query = (input.value || "").trim();
      apply(true);
    }, 80);
  });

  tagsWrap?.addEventListener("click", (e: MouseEvent) => {
    const btn = (e.target as Element | null)?.closest<HTMLButtonElement>(
      ".filter-tag:not(.filter-more)"
    );
    if (!btn) return;
    const tag = btn.dataset.tag || "";
    if (!tag) return;
    if (activeTags.has(tag)) activeTags.delete(tag);
    else activeTags.add(tag);
    apply(true);
  });

  // Story badge click-to-filter delegation scoped to this pane
  const badgeTarget: EventTarget = scope instanceof Element ? scope : document;
  badgeTarget.addEventListener("click", (e: Event) => {
    if (!isActive()) return;
    const badge = (e.target as Element | null)?.closest<HTMLElement>(
      ".badge[data-tag]"
    );
    if (!badge) return;
    const tag = badge.dataset.tag || "";
    if (!tag) return;
    e.preventDefault();
    activateTag(tag, true);
  });

  // Keyboard shortcuts: / to open, Esc (1st clear, 2nd close)
  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (!isActive()) return;
    if (e.key === "/" && !e.metaKey && !e.ctrlKey && !e.altKey) {
      if (mast.isTypingTarget(e.target)) return;
      e.preventDefault();
      open(true);
      return;
    }
    if (e.key === "Escape" && isDrawerOpen()) {
      e.preventDefault();
      if (query.length > 0 || activeTags.size > 0) {
        clear();
        input?.focus();
      } else {
        close(true);
      }
    }
  });

  // Restore ?q and ?tag from URL on load (active pane only)
  let restored = false;
  if (isActive()) {
    try {
      const params = new URLSearchParams(window.location.search);
      const qParam = (params.get("q") || "").trim();
      const tagParam = (params.get("tag") || "").trim();
      if (qParam) {
        query = qParam;
        if (input) input.value = qParam;
        restored = true;
      }
      if (tagParam) {
        for (const t of tagParam.split(",")) {
          const clean = t.trim();
          if (clean) activeTags.add(clean);
        }
        if (activeTags.size > 0) restored = true;
      }
    } catch {
      // Ignore malformed query string.
    }
  }

  apply(false);
  if (restored) {
    open(false);
  }

  return {
    open,
    close,
    clear,
    activateTag,
    isOpen: isDrawerOpen,
    refreshTags: layoutTags,
  };
}
