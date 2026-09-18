// Locale preference client module (runs in browsers only, not during SSR).
// - First visit: redirect to Chinese when navigator.languages starts with zh.
// - Explicit locale-menu picks are persisted and never auto-overridden.
// - Redirects keep baseUrl/path/query/hash and skip crawlers.

const STORAGE_KEY = 'mwc-locale';
const LOCALES = ['en', 'zh']; // en is default and lives at the unprefixed path
const DEFAULT_LOCALE = 'en';
// Visible labels from docusaurus.config.js locale configs; used to recognize
// actual locale selector links on both desktop dropdown and mobile sidebar.
const LOCALE_LABELS = { en: 'English', zh: '简体中文' };

function readSavedLocale() {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    return null; // storage blocked (private mode, policy): treat as unsaved
  }
}

function writeSavedLocale(locale) {
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // storage blocked: persistence is best-effort only
  }
}

function isCrawler() {
  if (navigator.webdriver) return true;
  return /bot|crawler|spider|slurp|headless/i.test(navigator.userAgent);
}

// Strip the configured baseUrl so path logic works on the site-relative path.
function stripBaseUrl(pathname) {
  const baseUrl = window.__docusaurus?.baseUrl ?? '/memeloop-workspace-control/';
  if (pathname.startsWith(baseUrl)) return pathname.slice(baseUrl.length);
  return pathname.replace(/^\//, '');
}

function localeOf(relativePath) {
  const first = relativePath.split('/')[0];
  return LOCALES.includes(first) && first !== DEFAULT_LOCALE ? first : DEFAULT_LOCALE;
}

function pathFor(relativePath, locale) {
  const rest = localeOf(relativePath) === DEFAULT_LOCALE
    ? relativePath
    : relativePath.slice(localeOf(relativePath).length + 1);
  return locale === DEFAULT_LOCALE ? rest : `${locale}/${rest}`;
}

function targetUrl(locale) {
  const relative = stripBaseUrl(window.location.pathname);
  const baseUrl = window.__docusaurus?.baseUrl ?? '/memeloop-workspace-control/';
  return `${baseUrl}${pathFor(relative, locale)}${window.location.search}${window.location.hash}`;
}

function pickInitialLocale() {
  const saved = readSavedLocale();
  if (saved) return null; // explicit choice: never override
  const primary = (navigator.languages && navigator.languages[0]) || navigator.language || '';
  if (primary.toLowerCase().startsWith('zh')) return 'zh';
  return null; // unsupported languages stay on English
}

// Persist a locale picked through the Docusaurus locale selector. Event
// delegation catches clicks on selector links before navigation happens.
// Only links whose visible label matches the configured locale labels are
// treated as explicit picks, so ordinary cross-locale content links do not
// count as a user choice.
function watchLocaleDropdown() {
  document.addEventListener('click', (event) => {
    const anchor = event.target.closest && event.target.closest('a[href]');
    if (!anchor) return;
    try {
      const url = new URL(anchor.href, window.location.href);
      if (url.origin !== window.location.origin) return;
      const locale = localeOf(stripBaseUrl(url.pathname));
      if (!LOCALES.includes(locale)) return;
      if (locale === localeOf(stripBaseUrl(window.location.pathname))) return;
      if (anchor.textContent.trim() !== LOCALE_LABELS[locale]) return;
      writeSavedLocale(locale);
    } catch {
      // ignore malformed hrefs
    }
  });
}

function main() {
  if (typeof window === 'undefined' || typeof document === 'undefined') return;
  if (isCrawler()) return;
  watchLocaleDropdown();
  const target = pickInitialLocale();
  if (!target) return;
  const current = localeOf(stripBaseUrl(window.location.pathname));
  if (current === target) return; // already localized: no redirect loop
  writeSavedLocale(target); // remember the auto-pick so it never loops
  window.location.replace(targetUrl(target));
}

if (typeof window !== 'undefined') {
  main();
}
