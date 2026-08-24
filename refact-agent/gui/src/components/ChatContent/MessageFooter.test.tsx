import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { ThemePropsContext } from "../Theme/ThemePropsContext";
import { MessageFooter } from "./MessageFooter";

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
});
