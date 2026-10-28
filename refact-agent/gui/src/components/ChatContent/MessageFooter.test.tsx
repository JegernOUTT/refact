import { readFileSync } from "node:fs";
import { join } from "node:path";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ThemePropsContext } from "../Theme/ThemePropsContext";
import { MessageFooter, MessageWrapper } from "./MessageFooter";
import styles from "./MessageFooter.module.css";

const footerCss = readFileSync(
  join(process.cwd(), "src/components/ChatContent/MessageFooter.module.css"),
  "utf8",
);

function rule(selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return footerCss.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`))?.[1] ?? "";
}

function renderFooter() {
  return render(
    <ThemePropsContext.Provider
      value={{ host: "web", themeProps: {}, appearance: "dark" }}
    >
      <MessageFooter
        usage={{
          prompt_tokens: 3,
          completion_tokens: 11,
          cache_read_input_tokens: 10_600,
          cache_creation_input_tokens: 5_400,
          total_tokens: 16_014,
        }}
      />
    </ThemePropsContext.Provider>,
  );
}

function renderFooterWithAliases() {
  return render(
    <ThemePropsContext.Provider
      value={{ host: "web", themeProps: {}, appearance: "dark" }}
    >
      <MessageFooter
        usage={{
          prompt_tokens: 3,
          completion_tokens: 11,
          cache_read_tokens: 10_600,
          cache_creation_tokens: 5_400,
          total_tokens: 16_014,
        }}
      />
    </ThemePropsContext.Provider>,
  );
}

describe("MessageFooter", () => {
  it("shows Anthropic cache usage token details", async () => {
    renderFooter();

    await userEvent.hover(screen.getByText("16.00k"));

    expect(await screen.findByText("Context size")).toBeInTheDocument();
    expect(screen.getByText("Cache read")).toBeInTheDocument();
    expect(screen.getByText("10.60k")).toBeInTheDocument();
    expect(screen.getByText("Cache creation")).toBeInTheDocument();
    expect(screen.getByText("5.40k")).toBeInTheDocument();

    const themeRoot = screen.getByText("This Message").closest(".radix-themes");
    expect(themeRoot?.getAttribute("data-appearance")).toBe("dark");
  });

  it("shows cache usage token details from legacy aliases", async () => {
    renderFooterWithAliases();

    await userEvent.hover(screen.getByText("16.00k"));

    expect(await screen.findByText("Context size")).toBeInTheDocument();
    expect(screen.getByText("Cache read")).toBeInTheDocument();
    expect(screen.getByText("10.60k")).toBeInTheDocument();
    expect(screen.getByText("Cache creation")).toBeInTheDocument();
    expect(screen.getByText("5.40k")).toBeInTheDocument();
  });

  it("renders a zero-height overlay lane for tool-only messages", () => {
    render(
      <MessageWrapper>
        <MessageFooter
          messageId="m1"
          onCopy={() => undefined}
          onBranch={() => undefined}
          onDelete={() => undefined}
          overlay
        />
      </MessageWrapper>,
    );
    const lane = screen.getByTestId("message-footer");
    expect(lane).toHaveClass(styles.footerLaneOverlay);
    expect(lane).not.toHaveClass(styles.footerLane);
    expect(lane).toHaveAttribute("data-overlay", "true");
  });

  it("keeps the in-flow lane by default", () => {
    render(<MessageFooter onCopy={() => undefined} />);
    const lane = screen.getByTestId("message-footer");
    expect(lane).toHaveClass(styles.footerLane);
    expect(lane).toHaveAttribute("data-overlay", "false");
  });

  it.each([false, true])(
    "lets Tab reach hidden footer buttons (overlay %s)",
    async (overlay) => {
      const user = userEvent.setup();
      render(
        <MessageWrapper>
          <MessageFooter
            messageId="m1"
            onCopy={() => undefined}
            onBranch={() => undefined}
            onDelete={() => undefined}
            overlay={overlay}
          />
        </MessageWrapper>,
      );
      const copy = screen.getByRole("button", { name: "Copy message" });
      const branch = screen.getByRole("button", { name: "Branch from here" });
      const remove = screen.getByRole("button", { name: "Delete message" });
      await user.tab();
      expect(copy).toHaveFocus();
      await user.tab();
      expect(branch).toHaveFocus();
      await user.tab();
      expect(remove).toHaveFocus();
    },
  );

  it("hides footer content without visibility so it stays focusable", () => {
    expect(footerCss).not.toMatch(/visibility\s*:\s*hidden/u);
    const content = rule(".footerContent");
    expect(content).toContain("opacity: 0");
    expect(content).toContain("pointer-events: none");
    const overlayLane = rule(".footerLaneOverlay");
    expect(overlayLane).toContain("position: absolute");
    expect(overlayLane).not.toContain("min-height");
    expect(footerCss).toMatch(
      /\.messageWrapper:focus-within \.footerContent,\s*\.messageWrapper:focus-visible \.footerContent\s*\{[^}]*opacity: 1;[^}]*pointer-events: auto;/u,
    );
  });
});
