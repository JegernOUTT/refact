import { describe, expect, test, vi } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { UserInput } from "./UserInput";
import styles from "./ChatContent.module.css";
import type { UserMessage } from "../../services/refact";

function renderUserInput(children: UserMessage["content"]) {
  return render(
    <UserInput messageIndex={0} onRetry={vi.fn()}>
      {children}
    </UserInput>,
  );
}

describe("UserInput", () => {
  test("clamps long text and expands without entering edit mode", async () => {
    const longText = Array.from(
      { length: 13 },
      (_, index) => `Line ${index + 1}`,
    ).join("\n");
    const { container, user } = renderUserInput(longText);

    const showMore = screen.getByRole("button", { name: "Show more" });
    const textBlock = container.querySelector(`.${styles.userInputText}`);

    expect(textBlock).toHaveClass(styles.userInputTextCollapsed);
    expect(showMore).toHaveAttribute("aria-expanded", "false");

    await user.click(showMore);

    expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(textBlock).not.toHaveClass(styles.userInputTextCollapsed);
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  test("does not render an expand toggle for short text", () => {
    renderUserInput("A short user message.");

    expect(
      screen.queryByRole("button", { name: /show more/i }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("A short user message.")).toBeInTheDocument();
  });

  test("keeps images outside the clamp behavior", () => {
    const imageUrl = "data:image/png;base64,abc123";
    const content: UserMessage["content"] = [
      { type: "text", text: "A short text with an image" },
      { type: "image_url", image_url: { url: imageUrl } },
    ];
    const { container } = renderUserInput(content);

    expect(
      screen.queryByRole("button", { name: /show more/i }),
    ).not.toBeInTheDocument();
    expect(
      container.querySelector(`img[src="${imageUrl}"]`),
    ).toBeInTheDocument();
  });

  test("keeps compressed messages on the Reveal path", () => {
    renderUserInput(`🗜️ ${"compressed ".repeat(120)}`);

    expect(screen.getByText("Click for more")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /show more/i }),
    ).not.toBeInTheDocument();
  });
});
