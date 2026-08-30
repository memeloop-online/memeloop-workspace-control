type OpenWindow = (url?: string | URL, target?: string, features?: string) => Window | null;

export function reserveWebShellWindow(
  openWindow: OpenWindow = window.open.bind(window),
): Window | null {
  // Open synchronously so browser popup protection permits the terminal tab. Passing the
  // `noopener` feature makes Chromium return null even when it created a tab, which leaves an
  // unreachable blank tab. Clear opener before awaiting the one-time ticket instead.
  const target = openWindow("about:blank", "_blank");
  if (target) target.opener = null;
  return target;
}
