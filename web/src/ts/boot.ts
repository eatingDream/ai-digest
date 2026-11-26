try {
  const t = localStorage.getItem("digest-theme");
  if (t === "light" || t === "dark") {
    document.documentElement.dataset.theme = t;
  }
  const dk =
    t === "dark" ||
    (t !== "light" &&
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-color-scheme: dark)").matches);
  const m = document.querySelector('meta[name="theme-color"]');
  if (m) m.setAttribute("content", dk ? "#141311" : "#faf8f4");
} catch {
  // Ignore storage / matchMedia restrictions in sandboxed contexts.
}
