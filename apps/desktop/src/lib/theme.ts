/** The theme the settings name: light, dark, or the system's. */
export function setTheme(theme: string) {
  if (theme === "light" || theme === "dark") document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
}
