import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, test, vi } from "vitest";

import { render } from "../../../utils/test-utils";
import { PageHeader } from "./PageHeader";

const NO_CONSOLE = { errors: 0, warnings: 0 };

describe("PageHeader", () => {
  test("renders the url and title chips", () => {
    render(
      <PageHeader
        console={NO_CONSOLE}
        title="Example Domain"
        url="https://example.com/docs"
      />,
    );

    expect(screen.getByText("https://example.com/docs")).toBeInTheDocument();
    expect(screen.getByText("Example Domain")).toBeInTheDocument();
    expect(screen.queryByTestId("browser-page-status")).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("browser-page-console"),
    ).not.toBeInTheDocument();
  });

  test("hides the status badge for a 2xx document", () => {
    render(
      <PageHeader
        console={NO_CONSOLE}
        status={204}
        url="https://example.com"
      />,
    );

    expect(screen.queryByTestId("browser-page-status")).not.toBeInTheDocument();
  });

  test("renders a neutral badge for a 3xx document", () => {
    render(
      <PageHeader
        console={NO_CONSOLE}
        status={301}
        url="https://example.com"
      />,
    );

    const badge = screen.getByTestId("browser-page-status");
    expect(badge).toHaveTextContent("HTTP 301");
    expect(badge).toHaveAttribute("data-status", "neutral");
  });

  test.each([404, 503])("renders an error badge for %i", (status) => {
    render(
      <PageHeader
        console={NO_CONSOLE}
        status={status}
        url="https://example.com"
      />,
    );

    const badge = screen.getByTestId("browser-page-status");
    expect(badge).toHaveTextContent(`HTTP ${status}`);
    expect(badge).toHaveAttribute("data-status", "error");
  });

  test("renders a console chip that toggles the console section", async () => {
    const user = userEvent.setup();
    const onToggleConsole = vi.fn();
    render(
      <PageHeader
        console={{ errors: 2, warnings: 1 }}
        consoleOpen={false}
        onToggleConsole={onToggleConsole}
        url="https://example.com"
      />,
    );

    const chip = screen.getByTestId("browser-page-console");
    expect(chip).toHaveTextContent("2 errors · 1 warning");
    expect(chip).toHaveAttribute("aria-expanded", "false");
    expect(chip).toHaveAttribute("data-tone", "danger");

    await user.click(chip);
    expect(onToggleConsole).toHaveBeenCalledTimes(1);
  });

  test("marks a warning-only console chip with the warning tone", () => {
    render(
      <PageHeader
        console={{ errors: 0, warnings: 3 }}
        url="https://example.com"
      />,
    );

    const chip = screen.getByTestId("browser-page-console");
    expect(chip).toHaveTextContent("3 warnings");
    expect(chip).toHaveAttribute("data-tone", "warning");
  });
});
