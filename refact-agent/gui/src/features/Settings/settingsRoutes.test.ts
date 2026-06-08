import { describe, expect, it } from "vitest";

import type { Page } from "../Pages/pagesSlice";
import type { SettingsSectionId } from "./settingsSections";
import { SETTINGS_SECTIONS } from "./settingsSections";
import { isSettingsPage, settingsPageToSection, settingsSectionToPage } from "./settingsRoutes";

const ALL_SECTION_IDS: SettingsSectionId[] = SETTINGS_SECTIONS.map((s) => s.id);

describe("isSettingsPage", () => {
  it("returns true for all 8 settings page names", () => {
    const settingsPages: Page[] = [
      { name: "general settings" },
      { name: "providers page" },
      { name: "default models" },
      { name: "customization" },
      { name: "integrations page" },
      { name: "scheduler" },
      { name: "documentation settings" },
      { name: "extensions" },
    ];
    for (const page of settingsPages) {
      expect(isSettingsPage(page), `expected isSettingsPage to be true for "${page.name}"`).toBe(true);
    }
  });

  it("returns false for non-settings pages", () => {
    const nonSettingsPages: Page[] = [
      { name: "chat" },
      { name: "history" },
      { name: "stats dashboard" },
      { name: "marketplace hub" },
      { name: "buddy" },
      { name: "knowledge graph" },
    ];
    for (const page of nonSettingsPages) {
      expect(isSettingsPage(page), `expected isSettingsPage to be false for "${page.name}"`).toBe(false);
    }
  });
});

describe("round-trip", () => {
  it("settingsPageToSection(settingsSectionToPage(id)) === id for every section id", () => {
    for (const id of ALL_SECTION_IDS) {
      const page = settingsSectionToPage(id);
      const roundTripped = settingsPageToSection(page);
      expect(roundTripped, `round-trip failed for section "${id}"`).toBe(id);
    }
  });
});

describe("settingsPageToSection", () => {
  it("returns the correct section id for each settings page name", () => {
    const cases: [Page, SettingsSectionId][] = [
      [{ name: "general settings" }, "general"],
      [{ name: "providers page" }, "providers"],
      [{ name: "default models" }, "models"],
      [{ name: "customization" }, "customization"],
      [{ name: "integrations page" }, "integrations"],
      [{ name: "scheduler" }, "scheduler"],
      [{ name: "documentation settings" }, "documentation"],
      [{ name: "extensions" }, "extensions"],
    ];
    for (const [page, expectedId] of cases) {
      expect(settingsPageToSection(page), `failed for "${page.name}"`).toBe(expectedId);
    }
  });

  it("returns null for non-settings pages", () => {
    const nonSettingsPages: Page[] = [
      { name: "chat" },
      { name: "history" },
      { name: "stats dashboard" },
    ];
    for (const page of nonSettingsPages) {
      expect(settingsPageToSection(page), `expected null for "${page.name}"`).toBeNull();
    }
  });
});
