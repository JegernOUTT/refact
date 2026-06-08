import type { Page } from "../Pages/pagesSlice";

export type MarketplaceTabId =
  | "skills"
  | "commands"
  | "subagents"
  | "mcp"
  | "extensions";

export function marketplaceTabToPage(tab: MarketplaceTabId): Page {
  switch (tab) {
    case "skills":
      return { name: "skills marketplace" };
    case "commands":
      return { name: "commands marketplace" };
    case "subagents":
      return { name: "subagents marketplace" };
    case "mcp":
      return { name: "mcp marketplace" };
    case "extensions":
      return { name: "marketplace hub", tab: "extensions" };
  }
}

export function marketplacePageToTab(page: Page): MarketplaceTabId | null {
  switch (page.name) {
    case "skills marketplace":
      return "skills";
    case "commands marketplace":
      return "commands";
    case "subagents marketplace":
      return "subagents";
    case "mcp marketplace":
      return "mcp";
    case "marketplace hub":
      return page.tab ?? "skills";
    default:
      return null;
  }
}

export function isMarketplacePage(page: Page): boolean {
  return marketplacePageToTab(page) !== null;
}
