try {
  const t = localStorage.getItem("digest-theme");
  const r = document.documentElement;
  if (t === "light" || t === "dark") {
    r.dataset.theme = t;
  }
  const f = localStorage.getItem("digest-font");
  if (f === "sm" || f === "md" || f === "lg") {
    r.dataset.font = f;
  }
  const dk =
    t === "dark" ||
    (t !== "light" &&
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);
  const cs = dk ? "dark" : "light";
  r.style.colorScheme = cs;
  const m = document.querySelector('meta[name="theme-color"]');
  if (m) m.setAttribute("content", dk ? __THEME_DARK_BG__ : __THEME_LIGHT_BG__);
  const c = document.querySelector('meta[name="color-scheme"]');
  if (c) c.setAttribute("content", cs);
} catch {
  // Ignore storage / matchMedia restrictions in sandboxed contexts.
}
