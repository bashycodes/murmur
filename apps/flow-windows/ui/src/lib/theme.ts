// Light or dark: the single class .lw-dark on <html> switches every token
// (theme.css). Follows Windows' app mode for now; a theme pref can set the
// class the same way later.

const media = window.matchMedia("(prefers-color-scheme: dark)");

function apply(): void {
  document.documentElement.classList.toggle("lw-dark", media.matches);
}

export function followSystemTheme(): void {
  apply();
  media.addEventListener("change", apply);
}
