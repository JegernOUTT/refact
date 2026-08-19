import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const readProjectFile = (path: string) =>
  readFileSync(join(process.cwd(), path), "utf8");

const chatCss = readProjectFile("src/components/Chat/Chat.module.css");
const contentCss = readProjectFile(
  "src/components/ChatContent/ChatContent.module.css",
);
const followButtonCss = readProjectFile(
  "src/components/ScrollArea/ScrollToBottomButton.module.css",
);

function rule(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))?.[1] ?? "";
}

describe("chat bottom dock clearance styles", () => {
  it("uses one measured variable for transcript and floating consumers", () => {
    // The transcript area must not reserve composer clearance anywhere in
    // Chat.module.css; the in-scroller spacer is the single source.
    expect(chatCss).not.toContain(
      "padding-bottom: var(--rf-composer-clearance",
    );
    expect(rule(contentCss, ".floatingLinks")).toContain(
      "bottom: var(--rf-composer-clearance, 0px)",
    );
    expect(rule(contentCss, ".composerClearance")).toContain(
      "height: calc(var(--rf-composer-clearance, 0px) + var(--rf-space-4))",
    );
    expect(rule(contentCss, ".queuedMessagesContainer")).toContain(
      "bottom: calc(var(--rf-composer-clearance, 0px) + var(--rf-space-2))",
    );
    expect(rule(followButtonCss, ".root")).toContain(
      "bottom: calc(var(--rf-composer-clearance, 0px) + var(--rf-space-4))",
    );
  });

  it("does not retain the legacy overlap variable or a fixed queue fallback", () => {
    const clearanceStyles = `${chatCss}\n${contentCss}\n${followButtonCss}`;

    expect(clearanceStyles).not.toContain("--rf-composer-overlap");
    expect(rule(contentCss, ".queuedMessagesContainer")).not.toContain("60px");
  });
});
