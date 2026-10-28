import type { BrowserActionResponse } from "../../../services/refact/browser";

export interface ReportTextBlock {
  label: string;
  text: string;
}

function pageState(report: BrowserActionResponse): string {
  return [
    report.title ? `Title: ${report.title}` : null,
    report.url ? `URL: ${report.url}` : null,
    `DOM stabilized: ${report.stabilized === false ? "No" : "Yes"}`,
  ]
    .filter((line): line is string => line !== null)
    .join("\n");
}

function context(report: BrowserActionResponse): string | null {
  const value = report.context;
  if (!value) return null;
  const identity = [
    value.viewport,
    value.locale,
    value.timezone,
    value.color_scheme,
  ].filter(Boolean);
  const permissions = value.permissions?.length
    ? `permissions: ${value.permissions.join(", ")}`
    : "permissions: none";
  return [
    identity.join(" · "),
    permissions,
    `cookies: ${value.cookie_count} · local storage: ${value.local_storage_count} · session storage: ${value.session_storage_count}`,
    value.offline ? "offline" : null,
    value.http_credentials ? "HTTP credentials: configured" : null,
  ]
    .filter(Boolean)
    .join("\n");
}

function dialogs(report: BrowserActionResponse): string | null {
  if (!report.dialogs?.length) return null;
  return report.dialogs
    .map((dialog) => {
      const handling = dialog.automatic
        ? `auto-${dialog.action}`
        : dialog.action;
      const defaultValue = dialog.default_value
        ? ` (default: ${dialog.default_value})`
        : "";
      return `[${dialog.type}] ${handling}: ${dialog.message}${defaultValue}`;
    })
    .join("\n");
}

function uploads(report: BrowserActionResponse): string | null {
  if (!report.uploads?.length) return null;
  return report.uploads
    .map((upload) => {
      const payload = upload.in_memory_payloads ? "in-memory" : "host paths";
      return `${upload.source}: ${upload.paths.join(", ")} (${payload})`;
    })
    .join("\n");
}

function newTabs(report: BrowserActionResponse): string | null {
  if (!report.new_tabs?.length) return null;
  return report.new_tabs
    .map((tab) => {
      const opener = tab.opener
        ? ` · opener ${tab.opener.tab_id}${
            tab.opener.frame_id ? ` frame ${tab.opener.frame_id}` : ""
          }`
        : "";
      const step =
        tab.opened_by_step === undefined || tab.opened_by_step === null
          ? ""
          : ` · step ${tab.opened_by_step + 1}`;
      return `${tab.active ? "Active" : "Opened"}: ${tab.title || tab.url}\n  ${
        tab.url
      }\n  ${tab.id}${opener}${step}`;
    })
    .join("\n");
}

function routes(report: BrowserActionResponse): string | null {
  if (!report.active_routes?.length) return null;
  return report.active_routes
    .map((route) => {
      const pattern =
        typeof route.pattern === "string"
          ? route.pattern
          : `/${route.pattern.source}/${route.pattern.flags ?? ""}`;
      return `${route.handler.type}: ${pattern}`;
    })
    .join("\n");
}

function interceptions(report: BrowserActionResponse): string | null {
  if (!report.intercepted_requests?.length) return null;
  return report.intercepted_requests
    .map((entry) => {
      const outcome = entry.status
        ? ` ${entry.status}`
        : entry.reason
          ? ` ${entry.reason}`
          : "";
      const redirect = entry.redirect_hop ? " · redirect hop" : "";
      return `${entry.action}${outcome}: ${entry.method} ${entry.url}${redirect}`;
    })
    .join("\n");
}

function locatorHandlers(report: BrowserActionResponse): string | null {
  if (!report.locator_handlers?.length) return null;
  return report.locator_handlers
    .map(
      (handler) =>
        `${handler.ok ? "Succeeded" : "Failed"} ${handler.name} · action: ${
          handler.action
        } · outcome: ${handler.outcome}`,
    )
    .join("\n");
}

export function reportTextBlocks(
  report: BrowserActionResponse,
): ReportTextBlock[] {
  const entries: [string, string | null][] = [
    ["Page state", pageState(report)],
    ["Context", context(report)],
    ["Locator handlers", locatorHandlers(report)],
    ["Dialogs", dialogs(report)],
    ["Uploads", uploads(report)],
    ["New tabs", newTabs(report)],
    ["Active routes", routes(report)],
    ["Intercepted requests", interceptions(report)],
  ];
  return entries.flatMap(([label, text]) => (text ? [{ label, text }] : []));
}
