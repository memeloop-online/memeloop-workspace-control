type OpenWindow = (url?: string | URL, target?: string, features?: string) => Window | null;

export function reserveWebShellWindow(
  openWindow: OpenWindow = window.open.bind(window),
  loadingLabel = "",
): Window | null {
  // Open synchronously so browser popup protection permits the terminal tab. Passing the
  // `noopener` feature makes Chromium return null even when it created a tab, which leaves an
  // unreachable blank tab. Clear opener before awaiting the one-time ticket instead.
  const target = openWindow("about:blank", "_blank");
  if (target) {
    target.opener = null;
    if (loadingLabel) {
      target.document.title = loadingLabel;
      target.document.body?.replaceChildren();
      const status = target.document.createElement("p");
      status.setAttribute("role", "status");
      status.style.cssText = "margin:0;position:absolute;inset:0;display:grid;place-items:center;font:14px/1.4 system-ui,sans-serif;color:#616161;";
      status.textContent = loadingLabel;
      target.document.body?.append(status);
    }
  }
  return target;
}
