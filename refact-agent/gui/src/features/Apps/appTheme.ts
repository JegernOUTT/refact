export const APP_THEME_TOKENS_STYLE_ID = "refact-app-theme-tokens";
export const APP_THEME_BASE_STYLE_ID = "refact-app-theme-base";

export const APP_THEME_TOKEN_NAMES: readonly string[] = [
  "--rf-bg",
  "--rf-color-fg",
  "--rf-color-muted",
  "--rf-color-faint",
  "--rf-surface-1",
  "--rf-surface-2",
  "--rf-surface-3",
  "--rf-surface-overlay",
  "--rf-scrim",
  "--rf-border",
  "--rf-border-strong",
  "--rf-color-accent",
  "--rf-color-accent-soft",
  "--rf-color-info",
  "--rf-selected-bg",
  "--rf-selected-border",
  "--rf-selected-fg",
  "--rf-color-success",
  "--rf-color-warning",
  "--rf-color-danger",
  "--rf-color-success-soft",
  "--rf-color-warning-soft",
  "--rf-color-danger-soft",
  "--rf-focus-ring",
  "--rf-focus-ring-w",
  "--rf-focus-ring-offset",
  "--rf-chart-1",
  "--rf-chart-2",
  "--rf-chart-3",
  "--rf-chart-4",
  "--rf-chart-5",
  "--rf-chart-6",
  "--rf-chart-7",
  "--rf-chart-8",
  "--rf-chart-grid",
  "--rf-chart-axis",
  "--rf-radius-chip",
  "--rf-radius-ctl",
  "--rf-radius-card",
  "--rf-radius-pill",
  "--rf-space-2xs",
  "--rf-space-xs",
  "--rf-space-1",
  "--rf-space-2",
  "--rf-space-3",
  "--rf-space-4",
  "--rf-space-5",
  "--rf-space-6",
  "--rf-text-1",
  "--rf-text-2",
  "--rf-text-3",
  "--rf-text-4",
  "--rf-text-5",
  "--rf-line-1",
  "--rf-line-2",
  "--rf-line-3",
  "--rf-line-4",
  "--rf-line-5",
  "--rf-weight-regular",
  "--rf-weight-medium",
  "--rf-weight-semibold",
  "--rf-weight-bold",
  "--rf-font-ui",
  "--rf-font-mono",
  "--rf-control-h",
  "--rf-control-h-sm",
  "--rf-control-h-lg",
  "--rf-hairline",
];

// Resolved ambient typography of the hosting pane (not tokens in the host).
export const APP_THEME_AMBIENT: Readonly<Record<string, string>> = {
  "--rf-app-font-family": "font-family",
  "--rf-app-font-size": "font-size",
  "--rf-app-line-height": "line-height",
};

export const APP_THEME_ALLOWED_NAMES: readonly string[] = [
  ...APP_THEME_TOKEN_NAMES,
  ...Object.keys(APP_THEME_AMBIENT),
];
const ALLOWED = new Set(APP_THEME_ALLOWED_NAMES);

export type AppThemeTokens = Record<string, string>;

export const APP_THEME_MAX_VALUE_CHARS = 240;
// No `;{}<>\@!:` or newlines, so a value cannot leave its declaration.
const SAFE_VALUE_RE = /^[A-Za-z0-9 #%(),.\-_'"/+*]+$/;
const UNSAFE_FUNCTION_RE =
  /(?:url|image|image-set|cross-fade|element|expression|attr|src|env)\s*\(/i;

export function isSafeAppThemeValue(value: unknown): value is string {
  if (typeof value !== "string") return false;
  if (value.length === 0 || value.length > APP_THEME_MAX_VALUE_CHARS)
    return false;
  if (!SAFE_VALUE_RE.test(value)) return false;
  if (value.includes("/*") || value.includes("*/")) return false;
  if (UNSAFE_FUNCTION_RE.test(value)) return false;
  if ((value.split('"').length - 1) % 2 !== 0) return false;
  if ((value.split("'").length - 1) % 2 !== 0) return false;
  let depth = 0;
  for (const ch of value) {
    if (ch === "(") depth += 1;
    else if (ch === ")") depth -= 1;
    if (depth < 0) return false;
  }
  return depth === 0;
}

export function sanitizeAppThemeTokens(input: unknown): AppThemeTokens {
  const out: AppThemeTokens = {};
  if (typeof input !== "object" || input === null || Array.isArray(input))
    return out;
  for (const [name, raw] of Object.entries(input)) {
    if (!ALLOWED.has(name) || typeof raw !== "string") continue;
    const value = raw.trim();
    if (isSafeAppThemeValue(value)) out[name] = value;
  }
  return out;
}

export function serializeAppThemeTokens(tokens: AppThemeTokens): string {
  const safe = sanitizeAppThemeTokens(tokens);
  const body = APP_THEME_ALLOWED_NAMES.filter((name) => name in safe)
    .map((name) => `${name}:${safe[name]};`)
    .join("");
  return `:root{${body}}`;
}

export function readAppThemeTokens(element: Element | null): AppThemeTokens {
  if (typeof window === "undefined") return {};
  const target = element ?? document.documentElement;
  const style = window.getComputedStyle(target);
  const raw: AppThemeTokens = {};
  for (const name of APP_THEME_TOKEN_NAMES) {
    const value = style.getPropertyValue(name);
    if (value) raw[name] = value;
  }
  for (const [name, property] of Object.entries(APP_THEME_AMBIENT)) {
    const value = style.getPropertyValue(property);
    if (value) raw[name] = value;
  }
  return sanitizeAppThemeTokens(raw);
}

export function sameAppThemeTokens(a: AppThemeTokens, b: AppThemeTokens) {
  const keys = Object.keys(a);
  return (
    keys.length === Object.keys(b).length && keys.every((k) => a[k] === b[k])
  );
}

// Zero-specificity defaults mirroring styles/base.css; authored CSS wins.
// The iframe is the viewport: no ambient body inset or overflow clamp. Document
// apps may opt into .refact-app-content (on body or a wrapper); full-height apps
// own their inner padding and scroll regions. A header plus 100vh still exceeds
// the viewport: use a shrinking flex/grid track for the remaining space instead.
export const APP_THEME_BASE_CSS = [
  ":where(*,*::before,*::after){box-sizing:border-box}",
  ":where(html){height:100%;background:var(--rf-bg,Canvas);color:var(--rf-color-fg,CanvasText);" +
    "font-family:var(--rf-app-font-family,var(--rf-font-ui,system-ui,sans-serif));" +
    "font-size:var(--rf-app-font-size,var(--rf-text-2));" +
    "line-height:var(--rf-app-line-height,var(--rf-line-2));" +
    "-webkit-text-size-adjust:100%;text-size-adjust:100%}",
  ":where(body){margin:0;min-height:100%;padding:0;overflow-wrap:anywhere}",
  ":where(.refact-app-content){padding:var(--rf-app-content-padding,var(--rf-space-3))}",
  ":where(html){--rf-app-scrollbar-size:var(--rf-space-2);" +
    "--rf-app-scrollbar-thumb:var(--rf-border-strong);--rf-app-scrollbar-track:var(--rf-bg);" +
    "--rf-app-scrollbar-thumb-hover:var(--rf-color-faint)}",
  ":where(*){scrollbar-width:thin;scrollbar-color:var(--rf-app-scrollbar-thumb) var(--rf-app-scrollbar-track)}",
  ":where(*):where(:hover){scrollbar-color:var(--rf-app-scrollbar-thumb-hover) var(--rf-app-scrollbar-track)}",
  ":where(*)::-webkit-scrollbar{width:var(--rf-app-scrollbar-size);height:var(--rf-app-scrollbar-size)}",
  ":where(*)::-webkit-scrollbar-track,:where(*)::-webkit-scrollbar-corner{background:var(--rf-app-scrollbar-track)}",
  ":where(*)::-webkit-scrollbar-thumb{background:var(--rf-app-scrollbar-thumb);border-radius:var(--rf-radius-pill)}",
  ":where(*)::-webkit-scrollbar-thumb:hover{background:var(--rf-app-scrollbar-thumb-hover)}",
  ":where(h1,h2,h3,h4,h5,h6){margin:0 0 var(--rf-space-2);font-weight:var(--rf-weight-semibold)}",
  ":where(h1){font-size:var(--rf-text-5);line-height:var(--rf-line-5)}",
  ":where(h2){font-size:var(--rf-text-4);line-height:var(--rf-line-4)}",
  ":where(h3){font-size:var(--rf-text-3);line-height:var(--rf-line-3)}",
  ":where(h4,h5,h6){font-size:var(--rf-text-2);line-height:var(--rf-line-2)}",
  ":where(p,ul,ol,dl,blockquote,figure,table,pre){margin:0 0 var(--rf-space-3)}",
  ":where(small){font-size:var(--rf-text-1);line-height:var(--rf-line-1)}",
  ":where(a){color:var(--rf-color-accent,LinkText);text-underline-offset:2px}",
  ":where(code,kbd,samp,pre){font-family:var(--rf-font-mono,monospace);font-size:var(--rf-text-1)}",
  ":where(pre){padding:var(--rf-space-2) var(--rf-space-3);background:var(--rf-surface-2);" +
    "border:var(--rf-hairline) solid var(--rf-border);border-radius:var(--rf-radius-ctl);overflow:auto}",
  ":where(hr){border:0;border-top:var(--rf-hairline) solid var(--rf-border);margin:var(--rf-space-3) 0}",
  ":where(table){border-collapse:collapse;width:100%}",
  ":where(th,td){padding:var(--rf-space-1) var(--rf-space-2);text-align:start;" +
    "border-bottom:var(--rf-hairline) solid var(--rf-border)}",
  ":where(th){color:var(--rf-color-muted);font-weight:var(--rf-weight-medium)}",
  ":where(button,input,select,textarea){font:inherit;color:inherit;margin:0}",
  ":where(button,input,select,textarea):where(:not([type=checkbox],[type=radio],[type=range],[type=color])){" +
    "min-height:var(--rf-control-h);border:var(--rf-hairline) solid var(--rf-border);" +
    "border-radius:var(--rf-radius-ctl);background:var(--rf-surface-1)}",
  ":where(input,select,textarea,progress,meter){max-width:100%;min-width:0}",
  ":where(input:not([type=checkbox],[type=radio],[type=range],[type=color]),select,textarea){padding:var(--rf-space-1) var(--rf-space-2)}",
  ":where(input[type=checkbox],input[type=radio]){padding:0;vertical-align:middle}",
  ":where(input[type=range]){padding:0;vertical-align:middle;cursor:pointer}",
  ":where(input[type=file])::file-selector-button{font:inherit;color:inherit;" +
    "padding:var(--rf-space-1) var(--rf-space-2);margin-inline-end:var(--rf-space-2);" +
    "border:var(--rf-hairline) solid var(--rf-border);border-radius:var(--rf-radius-chip);" +
    "background:var(--rf-surface-2);cursor:pointer}",
  ":where(progress,meter){vertical-align:middle;accent-color:var(--rf-color-accent)}",
  ":where(progress)::-webkit-progress-bar{background:var(--rf-surface-2);border-radius:var(--rf-radius-pill)}",
  ":where(progress)::-webkit-progress-value{background:var(--rf-color-accent);border-radius:var(--rf-radius-pill)}",
  ":where(progress)::-moz-progress-bar{background:var(--rf-color-accent);border-radius:var(--rf-radius-pill)}",
  ":where(summary){cursor:pointer;font-weight:var(--rf-weight-medium)}",
  ":where(summary)::marker{color:var(--rf-color-muted)}",
  ":where(textarea){min-height:calc(var(--rf-control-h) * 2);resize:vertical}",
  ":where(button){padding:0 var(--rf-space-3);background:var(--rf-surface-2);cursor:pointer;" +
    "font-weight:var(--rf-weight-medium)}",
  ":where(button:hover:not(:disabled)){background:var(--rf-surface-3);border-color:var(--rf-border-strong)}",
  ":where(button:disabled,input:disabled,select:disabled,textarea:disabled){color:var(--rf-color-faint);cursor:not-allowed}",
  ":where(input,textarea)::placeholder{color:var(--rf-color-faint)}",
  ":where(input[type=checkbox],input[type=radio],input[type=range],progress){accent-color:var(--rf-color-accent)}",
  ":where(:focus-visible){outline:var(--rf-focus-ring-w) solid var(--rf-focus-ring);outline-offset:var(--rf-focus-ring-offset)}",
  "::selection{background:var(--rf-color-accent-soft)}",
].join("");
