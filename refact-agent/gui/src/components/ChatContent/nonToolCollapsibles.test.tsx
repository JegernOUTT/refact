import type React from "react";
import { Theme } from "@radix-ui/themes";
import { fireEvent, render, screen } from "@testing-library/react";
import { Provider } from "react-redux";
import { describe, expect, it, vi } from "vitest";

import { ContextFiles } from "./ContextFiles";
import { SystemPrompt } from "./SystemPrompt";
import { ContextFileList } from "./ToolCard/ContextFileList";
import type { ChatContextFile } from "../../services/refact/types";
import { setUpStore } from "../../app/store";

vi.mock("../Markdown", () => ({
  Markdown: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));

vi.mock("../Markdown/ShikiCodeBlock", () => ({
  ShikiCodeBlock: ({ children }: { children: React.ReactNode }) => (
    <pre>{children}</pre>
  ),
}));

vi.mock("../Markdown", () => ({
  ShikiCodeBlock: ({ children }: { children: React.ReactNode }) => (
    <pre>{children}</pre>
  ),
}));

vi.mock("../../hooks", () => ({
  useAppDispatch: () => vi.fn(),
  useEventsBusForIDE: () => ({ queryPathThenOpenFile: vi.fn() }),
}));

vi.mock("../../../hooks", () => ({
  useEventsBusForIDE: () => ({ queryPathThenOpenFile: vi.fn() }),
}));

const file: ChatContextFile = {
  file_name: "src/example.ts",
  file_content: "export const value = 1;",
  line1: 1,
  line2: 3,
};

function renderWithTheme(ui: React.ReactElement) {
  return render(
    <Provider store={setUpStore()}>
      <Theme>{ui}</Theme>
    </Provider>,
  );
}

describe("non-tool chat collapsibles", () => {
  it("renders SystemPrompt header as an aria-expanded button", () => {
    renderWithTheme(<SystemPrompt content="System content" />);

    const trigger = screen.getByRole("button", { name: /system prompt/i });
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(trigger);

    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("System content")).toBeInTheDocument();
  });

  it("renders ContextFiles header as an aria-expanded button", () => {
    renderWithTheme(<ContextFiles files={[file]} />);

    const trigger = screen.getByRole("button", { name: /example\.ts:1-3/i });
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(trigger);

    expect(trigger).toHaveAttribute("aria-expanded", "true");
  });

  it("renders ContextFileList item header as an aria-expanded button", () => {
    renderWithTheme(<ContextFileList files={[file]} />);

    const trigger = screen.getByRole("button", { name: /example\.ts:1-3/i });
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    fireEvent.click(trigger);

    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("export const value = 1;")).toBeInTheDocument();
  });
});
