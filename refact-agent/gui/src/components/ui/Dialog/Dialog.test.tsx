import { readFileSync } from "node:fs";
import { join } from "node:path";
import React from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { Dialog } from ".";

const tokensCss = readFileSync(
  join(process.cwd(), "src/styles/tokens.css"),
  "utf8",
);
const dialogCss = readFileSync(
  join(process.cwd(), "src/components/ui/Dialog/Dialog.module.css"),
  "utf8",
);

const tokenValue = (name: string) => {
  const match = tokensCss.match(new RegExp(`${name}:\\s*(\\d+);`));
  return match ? Number(match[1]) : Number.NaN;
};

describe("Dialog", () => {
  it("stacks modal content above its overlay", () => {
    expect(tokenValue("--rf-z-overlay")).toBeLessThan(
      tokenValue("--rf-z-modal"),
    );
    expect(dialogCss).toMatch(
      /\.overlay[\s\S]*z-index:\s*var\(--rf-z-overlay, 600\)/,
    );
    expect(dialogCss).toMatch(
      /\.content[\s\S]*z-index:\s*var\(--rf-z-modal, 700\)/,
    );
  });

  it("clamps height to the viewport without an invalid min(auto) fallback", () => {
    expect(dialogCss).not.toContain("var(--rf-overlay-max-height, auto)");
    expect(dialogCss).toContain(
      "--rf-overlay-viewport-max-height: calc(100dvh - var(--rf-space-5))",
    );
    expect(dialogCss).toMatch(
      /max-height:\s*min\(\s*var\(--rf-overlay-ideal-height\),\s*var\(--rf-overlay-viewport-max-height\)/,
    );
  });
});

describe("Dialog content partitioning", () => {
  const openDialog = (children: React.ReactNode) =>
    render(
      <Dialog open>
        <Dialog.Content aria-label="Partition test">{children}</Dialog.Content>
      </Dialog>,
    );

  afterEach(() => {
    cleanup();
  });

  it("pins a leading Title/Description run in the header region", () => {
    openDialog(
      <>
        <Dialog.Title>Head</Dialog.Title>
        <Dialog.Description>Sub</Dialog.Description>
        <p>Body copy</p>
      </>,
    );
    const title = screen.getByText("Head");
    const header = title.parentElement;
    expect(header?.className).toContain("header");
    expect(screen.getByText("Sub").parentElement).toBe(header);
    expect(screen.getByText("Body copy").parentElement?.className).toContain(
      "inner",
    );
  });

  it("pins a bare trailing Dialog.Close in the footer region", () => {
    openDialog(
      <>
        <p>Body copy</p>
        <Dialog.Close>Close</Dialog.Close>
      </>,
    );
    const close = screen.getByRole("button", { name: "Close" });
    expect(close.parentElement?.className).toContain("footer");
  });

  it("pins wrapped actions declared through Dialog.Footer", () => {
    openDialog(
      <>
        <p>Body copy</p>
        <Dialog.Footer>
          <button type="button">Cancel</button>
          <Dialog.Close>Confirm</Dialog.Close>
        </Dialog.Footer>
      </>,
    );
    const row = screen.getByRole("button", { name: "Cancel" }).parentElement;
    expect(row?.className).toContain("footerRow");
    expect(row?.parentElement?.className).toContain("footer");
    expect(screen.getByText("Body copy").parentElement?.className).toContain(
      "inner",
    );
  });

  it("keeps interleaved markup in document order inside the body", () => {
    openDialog(
      <>
        <Dialog.Title>Head</Dialog.Title>
        <p>First</p>
        <Dialog.Close>Middle close</Dialog.Close>
        <p>Last</p>
      </>,
    );
    const body = screen.getByText("First").parentElement;
    expect(body?.className).toContain("inner");
    expect(
      screen.getByRole("button", { name: "Middle close" }).parentElement,
    ).toBe(body);
    expect(screen.getByText("Last").parentElement).toBe(body);
  });

  it("forwards arbitrary content props to the dialog node", () => {
    openDialog(<p>Body copy</p>);
    expect(
      screen.getByRole("dialog", { name: "Partition test" }),
    ).toBeInTheDocument();
  });
});
