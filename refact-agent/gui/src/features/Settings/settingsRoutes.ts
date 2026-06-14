import type { Page } from "../Pages/pagesSlice";
import type { SettingsSectionId } from "./settingsSections";

export function settingsSectionToPage(section: SettingsSectionId): Page {
  switch (section) {
    case "general":
      return { name: "general settings" };
    case "providers":
      return { name: "providers page" };
    case "models":
      return { name: "default models" };
    case "customization":
      return { name: "customization" };
    case "integrations":
      return { name: "integrations page" };
    case "scheduler":
      return { name: "scheduler" };
    case "extensions":
      return { name: "extensions" };
    case "marketplace":
      return { name: "marketplace hub" };
  }
}

export function settingsPageToSection(page: Page): SettingsSectionId | null {
  switch (page.name) {
    case "general settings":
      return "general";
    case "providers page":
      return "providers";
    case "default models":
      return "models";
    case "customization":
      return "customization";
    case "integrations page":
      return "integrations";
    case "scheduler":
      return "scheduler";
    case "extensions":
      return "extensions";
    case "marketplace hub":
    case "skills marketplace":
    case "commands marketplace":
    case "subagents marketplace":
    case "mcp marketplace":
      return "marketplace";
    default:
      return null;
  }
}

export function isSettingsPage(page: Page): boolean {
  return settingsPageToSection(page) !== null;
}
