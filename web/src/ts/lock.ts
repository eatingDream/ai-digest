const STORE_KEY = "aie.k";

document.addEventListener("click", (e: MouseEvent) => {
  const btn = (e.target as Element | null)?.closest?.("[data-lock]");
  if (!btn) return;
  e.preventDefault();
  try {
    sessionStorage.removeItem(STORE_KEY);
  } catch {
    // Ignore storage access errors.
  }
  try {
    localStorage.removeItem(STORE_KEY);
  } catch {
    // Ignore storage access errors.
  }
  location.reload();
});
