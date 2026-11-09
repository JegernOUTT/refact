import { appCaptureRuntime } from "./appCapture";
import {
  APP_SDK_VERSION,
  appSdkRuntime,
  type AppAppearance,
  type AppSdkRuntimeConfig,
} from "./appSdk";
import {
  APP_THEME_BASE_CSS,
  APP_THEME_BASE_STYLE_ID,
  APP_THEME_TOKENS_STYLE_ID,
  serializeAppThemeTokens,
  type AppThemeTokens,
} from "./appTheme";

export const APP_IFRAME_SANDBOX = "allow-scripts";
export const APP_IFRAME_ALLOW = "";
export const APP_IFRAME_REFERRER_POLICY = "no-referrer";
export const APP_MAX_HTML_CHARS = 5_000_000;

export const APP_DOCUMENT_CSP = [
  "default-src 'none'",
  "script-src 'unsafe-inline'",
  "style-src 'unsafe-inline'",
  "img-src data: blob:",
  "font-src data:",
  "media-src data: blob:",
  "connect-src 'none'",
  "frame-src 'none'",
  "child-src 'none'",
  "worker-src 'none'",
  "object-src 'none'",
  "manifest-src 'none'",
  "base-uri 'none'",
  "form-action 'none'",
].join("; ");

export type BuildAppDocumentOptions = {
  html: string;
  channel: string;
  generation: number;
  appearance?: AppAppearance;
  /** Allowlisted `--rf-*` tokens; sanitized again before serialization. */
  themeTokens?: AppThemeTokens;
  initialState?: unknown;
  title?: string;
};

export class AppDocumentError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AppDocumentError";
  }
}

const escapeHtml = (s: string) =>
  s.replace(/[&<>"']/g, (c) => `&#${String(c.charCodeAt(0))};`);

export function serializeForScript(value: unknown): string {
  const json = JSON.stringify(value);
  return (json || "null")
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/&/g, "\\u0026")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
}

export function buildAppSdkScript(config: AppSdkRuntimeConfig): string {
  const sdk = appSdkRuntime.toString();
  const capture = appCaptureRuntime.toString();
  const src = `"use strict";(${sdk})(${serializeForScript(
    config,
  )},${capture});`;
  if (/<\/script|<!--/i.test(src)) {
    throw new AppDocumentError("SDK source contains unsafe sequence");
  }
  return src;
}

const CHANNEL_RE = /^[A-Za-z0-9_-]{8,128}$/;

export function buildAppDocument(options: BuildAppDocumentOptions): string {
  const { html, channel, generation } = options;
  if (typeof html !== "string")
    throw new AppDocumentError("html must be a string");
  if (html.length > APP_MAX_HTML_CHARS)
    throw new AppDocumentError("html too large");
  if (!CHANNEL_RE.test(channel)) throw new AppDocumentError("invalid channel");
  if (!Number.isSafeInteger(generation) || generation < 0) {
    throw new AppDocumentError("invalid generation");
  }
  const appearance: AppAppearance =
    options.appearance === "dark" ? "dark" : "light";
  const script = buildAppSdkScript({
    channel,
    generation,
    appearance,
    initialState: options.initialState ?? null,
    sdkVersion: APP_SDK_VERSION,
  });
  const body = html.replace(/^\uFEFF?\s*<!doctype[^>]*>/i, "");
  const title = options.title
    ? `<title>${escapeHtml(options.title.slice(0, 200))}</title>`
    : "";
  const tokens = serializeAppThemeTokens(options.themeTokens ?? {});
  if (/[<>]/.test(tokens) || /[<>]/.test(APP_THEME_BASE_CSS)) {
    throw new AppDocumentError("theme styles contain unsafe sequence");
  }
  return (
    `<!doctype html><html data-refact-appearance="${appearance}" style="color-scheme:${appearance}"><head>` +
    `<meta http-equiv="Content-Security-Policy" content="${APP_DOCUMENT_CSP}">` +
    `<meta name="referrer" content="no-referrer">` +
    `<meta charset="utf-8">` +
    `<meta name="viewport" content="width=device-width, initial-scale=1">` +
    `<meta name="color-scheme" content="${appearance}">` +
    title +
    `<style id="${APP_THEME_BASE_STYLE_ID}">${APP_THEME_BASE_CSS}</style>` +
    `<style id="${APP_THEME_TOKENS_STYLE_ID}">${tokens}</style>` +
    `<script>${script}</script>` +
    `</head>` +
    body
  );
}
