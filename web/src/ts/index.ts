import { initFilter } from "./filter";
import { initMast } from "./mast";

function initArchive(): void {
  const mast = initMast();
  const items = Array.from(
    document.querySelectorAll<HTMLElement>("li.archive-item")
  );
  const groups = Array.from(
    document.querySelectorAll<HTMLDetailsElement>("details.grp")
  );
  const months = Array.from(
    document.querySelectorAll<HTMLElement>("section.month-group")
  );

  const initialOpen = new Map<HTMLDetailsElement, boolean>();
  for (const grp of groups) {
    initialOpen.set(grp, grp.open);
  }

  const normSearch = (s: string): string => s.replace(/\u2011/g, "-").toLowerCase();
  const cache = new Map<HTMLElement, { search: string; tags: Set<string> }>();
  for (const item of items) {
    const rawTags = (item.dataset.tags || "")
      .split("|")
      .map((t) => t.trim())
      .filter(Boolean);
    const search = normSearch(
      `${item.dataset.search || ""} ${item.textContent || ""} ${rawTags.join(" ")}`
    );
    cache.set(item, { search, tags: new Set(rawTags) });
  }

  initFilter({
    mast,
    onFilter: (query, activeTags, _activeCat) => {
      const qLow = normSearch(query);
      const hasFilter = qLow.length > 0 || activeTags.size > 0;
      let matched = 0;

      for (const item of items) {
        const meta = cache.get(item);
        let ok = true;
        if (meta) {
          for (const t of activeTags) {
            if (!meta.tags.has(t)) {
              ok = false;
              break;
            }
          }
          if (ok && qLow && !meta.search.includes(qLow)) ok = false;
        }
        item.hidden = !ok;
        if (ok) matched++;
      }

      for (const grp of groups) {
        const grpItems = Array.from(
          grp.querySelectorAll<HTMLElement>("li.archive-item")
        );
        const vis = grpItems.filter((it) => !it.hidden).length;
        grp.hidden = vis === 0;
        if (hasFilter) {
          if (vis > 0) grp.open = true;
        } else {
          grp.open = Boolean(initialOpen.get(grp));
        }
      }

      for (const m of months) {
        const mGroups = Array.from(
          m.querySelectorAll<HTMLDetailsElement>("details.grp")
        );
        const anyVis = mGroups.some((g) => !g.hidden);
        m.hidden = !anyVis;
      }

      return { matched, total: items.length };
    },
  });
}

initArchive();
