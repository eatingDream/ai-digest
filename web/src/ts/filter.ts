import type { MastController } from "./mast";

export interface FilterResult {
  matched: number;
  total: number;
}

export interface FilterOptions {
  mast: MastController;
  onFilter: (query: string, activeTags: Set<string>) => FilterResult;
  onFirstMatchScroll?: () => void;
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

export function initFilter(opts: FilterOptions): FilterController {
  const { mast, onFilter, onFirstMatchScroll } = opts;
  const scope: ParentNode = opts.root ?? document;
  const isActive = opts.isActive ?? (() => true);
  const toggleBtn = scope.querySelector<HTMLButtonElement>("[data-filter-toggle]");
  const drawer =
    scope.querySelector<HTMLElement>(".filter-drawer") ??
    scope.querySelector<HTMLElement>("#digestFilterBar");
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

  // Snapshot original tag buttons in count-desc order
  const tagButtons: HTMLButtonElement[] = tagsWrap
    ? Array.from(tagsWrap.querySelectorAll<HTMLButtonElement>(".filter-tag:not(.filter-more)"))
    : [];
  tagButtons.forEach((btn, idx) => {
    btn.dataset.origIdx = String(idx);
  });

  // Dynamic "+N more" / "Less" toggle button
  let moreBtn = tagsWrap?.querySelector<HTMLButtonElement>(".filter-more") ?? null;
  if (tagsWrap && !moreBtn && tagButtons.length > 0) {
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

  // Wrapping 2-row collapse with dynamic "+N" placed as last visible slot (never clips tags)
  const layoutTags = (): void => {
    if (!tagsWrap || !moreBtn || tagButtons.length === 0) return;
    if (drawer?.hidden || !isActive()) return;

    const ordered = Array.from(
      tagsWrap.querySelectorAll<HTMLButtonElement>(".filter-tag:not(.filter-more)")
    );
    for (const btn of ordered) btn.hidden = false;

    if (expandedTags) {
      tagsWrap.classList.add("is-expanded");
      moreBtn.hidden = false;
      moreBtn.textContent = labelLess();
      moreBtn.setAttribute("aria-expanded", "true");
      tagsWrap.appendChild(moreBtn);
      return;
    }

    tagsWrap.classList.remove("is-expanded");
    moreBtn.hidden = true;

    // Measure distinct row Y offsets across all tags
    const rowTops: number[] = [];
    for (const btn of ordered) {
      const top = btn.offsetTop;
      if (!rowTops.some((r) => Math.abs(r - top) <= 3)) {
        rowTops.push(top);
      }
    }
    if (rowTops.length <= 2) {
      moreBtn.hidden = true;
      return;
    }

    const row2Top = rowTops[1];
    let hiddenCount = 0;
    let firstOverflowIdx = ordered.length;
    for (let i = 0; i < ordered.length; i++) {
      if (ordered[i].offsetTop > row2Top + 3) {
        firstOverflowIdx = i;
        break;
      }
    }

    for (let i = firstOverflowIdx; i < ordered.length; i++) {
      ordered[i].hidden = true;
      hiddenCount++;
    }

    // Place "+N" as last visible slot in row 2, hiding extra row-2 tags if "+N" wraps to row 3
    moreBtn.hidden = false;
    moreBtn.setAttribute("aria-expanded", "false");
    let visibleEnd = firstOverflowIdx;
    while (visibleEnd > 0) {
      moreBtn.textContent = labelMore(hiddenCount);
      tagsWrap.insertBefore(moreBtn, ordered[visibleEnd] || null);
      if (moreBtn.offsetTop <= row2Top + 3) break;
      visibleEnd--;
      ordered[visibleEnd].hidden = true;
      hiddenCount++;
    }
    moreBtn.textContent = labelMore(hiddenCount);
  };

  moreBtn?.addEventListener("click", () => {
    expandedTags = !expandedTags;
    layoutTags();
  });

  if (tagsWrap && typeof ResizeObserver !== "undefined") {
    const ro = new ResizeObserver(() => layoutTags());
    ro.observe(tagsWrap);
  }

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
    toggleBtn?.setAttribute("aria-expanded", "true");
    mast.setPinned("filter", true);
    layoutTags();
    if (focusInput && input) {
      input.focus();
      input.select();
    }
  };

  const close = (restoreFocus = true): void => {
    if (!drawer) return;
    drawer.hidden = true;
    toggleBtn?.setAttribute("aria-expanded", "false");
    mast.setPinned("filter", false);
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
    if (drawer?.hidden) open(true);
    else close(true);
  });

  closeBtn?.addEventListener("click", () => close(true));

  for (const btn of clearBtns) {
    btn.addEventListener("click", () => {
      clear();
      if (!drawer?.hidden) input?.focus();
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
    if (e.key === "Escape" && drawer && !drawer.hidden) {
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
    isOpen: () => Boolean(drawer && !drawer.hidden),
    refreshTags: layoutTags,
  };
}
