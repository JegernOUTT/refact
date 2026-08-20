import { describe, expect, test } from "vitest";
import { readFileSync } from "fs";
import { join } from "path";

/**
 * Tap-target floor enforcement (audit N-51).
 *
 * The kit documents 28px (--rf-control-h-icon-sm) as the minimum interactive
 * target. Every historically sub-floor control was fixed in the remediation
 * waves; these characterization tests keep the fixes from regressing.
 * Undersized visuals must keep an expanded ::before hit area.
 */

const GUI_SRC = join(__dirname, "..");

function css(relPath: string): string {
  return readFileSync(join(GUI_SRC, relPath), "utf8");
}

function block(text: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = new RegExp(`${escaped}\\s*\\{[^}]*\\}`, "g").exec(text);
  return match?.[0] ?? "";
}

describe("tap-target floor (N-51)", () => {
  test("tokens define the 28px icon-sm floor", () => {
    const tokens = css("styles/tokens.css");
    expect(tokens).toContain("--rf-control-h-icon-sm: 28px");
  });

  test("Chip keeps the icon-sm min-height floor and remove-button hit area", () => {
    const chip = css("components/ui/Chip/Chip.module.css");
    expect(block(chip, ".chip")).toContain(
      "min-height: var(--rf-control-h-icon-sm)",
    );
    const removeBefore = block(chip, ".remove::before");
    expect(removeBefore).toContain(
      "inset: calc((var(--rf-control-h-icon-sm) - var(--rf-icon-lg)) / -2)",
    );
    expect(removeBefore).toContain('content: ""');
  });

  test("DialogImage inline trigger keeps its expanded hit area (N-32)", () => {
    const dialogImage = css("components/DialogImage/DialogImage.module.css");
    const triggerBefore = block(dialogImage, ".trigger::before");
    expect(triggerBefore).toContain('content: ""');
    expect(triggerBefore).toContain("inset: calc(-1 * var(--rf-space-xs))");
  });

  test("ModelSelector search input stays on the icon-sm rung (N-41)", () => {
    const modelSelector = css(
      "components/ui/ModelSelector/ModelSelector.module.css",
    );
    expect(block(modelSelector, ".searchInput")).toContain(
      "height: var(--rf-control-h-icon-sm)",
    );
  });

  test("diff controls stay on the control ladder (N-29)", () => {
    const editTool = css("components/ChatContent/ToolCard/EditTool.module.css");
    expect(block(editTool, ".hunkFileButton")).toContain(
      "min-height: var(--rf-control-h-sm)",
    );
    expect(block(editTool, ".showMoreButton")).toContain(
      "min-height: var(--rf-control-h-sm)",
    );
  });

  test("PlanBanner controls stay on the icon-sm rung (N-34)", () => {
    const planBanner = css(
      "components/ChatContent/PlanBanner/PlanBanner.module.css",
    );
    expect(block(planBanner, ".toggleButton")).toContain(
      "min-height: var(--rf-control-h-icon-sm)",
    );
    const action = block(planBanner, ".actionButton");
    expect(action).toContain("width: var(--rf-control-h-icon-sm)");
    expect(action).toContain("height: var(--rf-control-h-icon-sm)");
  });
});
