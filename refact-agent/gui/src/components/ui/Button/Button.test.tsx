import { readFile } from "node:fs/promises";
import path from "node:path";

import { ChevronRight, ExternalLink, Plus } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { Button, IconButton } from "./Button";
import styles from "./Button.module.css";

describe("Button", () => {
  it("renders a native button by default", () => {
    render(
      <Button leftIcon={Plus} variant="primary">
        Create
      </Button>,
    );

    const button = screen.getByRole("button", { name: "Create" });

    expect(button).toBeInTheDocument();
    expect(button).toHaveAttribute("type", "button");
    expect(button).toHaveClass("rf-pressable");
  });

  it("renders text-only content without icon placeholders", () => {
    render(<Button>Create</Button>);

    const button = screen.getByRole("button", { name: "Create" });
    const label = button.querySelector("span");

    expect(button.children).toHaveLength(1);
    expect(label).toHaveTextContent("Create");
    expect(label?.children).toHaveLength(0);
  });

  it("does not reserve a global scrollbar gutter inside label-sized controls", async () => {
    const css = await readFile(
      path.resolve(__dirname, "Button.module.css"),
      "utf8",
    );
    const sharedControl =
      css.match(/\.button,\n\.iconButton \{[^}]+\}/)?.[0] ?? "";
    const label = css.match(/\.label \{[^}]+\}/)?.[0] ?? "";

    render(<Button>Preview</Button>);

    expect(screen.getByRole("button", { name: "Preview" })).toHaveClass(
      styles.button,
    );
    expect(sharedControl).toContain("display: inline-flex;");
    expect(sharedControl).toContain("scrollbar-gutter: auto;");
    expect(label).toContain("scrollbar-gutter: auto;");
  });

  it("renders rightIcon after the label without extra placeholders", () => {
    render(<Button rightIcon={ChevronRight}>Next</Button>);

    const button = screen.getByRole("button", { name: "Next" });
    const label = button.children.item(0);
    const icon = button.children.item(1);

    expect(button.children).toHaveLength(2);
    expect(label).toHaveTextContent("Next");
    expect(icon).toHaveAttribute("aria-hidden", "true");
    expect(icon?.querySelector("svg")).toBeInTheDocument();
  });

  it("renders loading state with spinner and label only", () => {
    render(<Button loading>Saving</Button>);

    const button = screen.getByRole("button", { name: "Saving" });
    const spinner = button.children.item(0);
    const label = button.children.item(1);

    expect(button).toBeDisabled();
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(button.children).toHaveLength(2);
    expect(spinner).toHaveAttribute("aria-hidden", "true");
    expect(spinner?.querySelector("svg")).toHaveClass("lucide-loader-circle");
    expect(label).toHaveTextContent("Saving");
  });

  it("renders icon-only Button without an empty label", () => {
    render(
      <Button aria-label="Add item">
        <Plus />
      </Button>,
    );

    const button = screen.getByRole("button", { name: "Add item" });

    const icon = button.children.item(0);

    expect(button).toHaveClass(styles.iconOnly);
    expect(button.children).toHaveLength(1);
    expect(icon).toHaveAttribute("aria-hidden", "true");
    expect(icon).not.toHaveTextContent("Add item");
    expect(button.querySelector("svg")).toHaveClass("lucide-plus");
  });

  it("composes props and content onto an asChild element", () => {
    render(
      <Button
        asChild
        className="outer-class"
        leftIcon={ExternalLink}
        variant="outline"
      >
        <a
          className="child-class"
          href="https://example.com"
          target="_blank"
          rel="noreferrer"
        >
          Open docs
        </a>
      </Button>,
    );

    const link = screen.getByRole("link", { name: "Open docs" });

    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(link).toHaveAttribute("href", "https://example.com");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveClass("outer-class");
    expect(link).toHaveClass("child-class");
    expect(link).toHaveClass("rf-pressable");
  });

  it("keeps asChild link icon and text inside one label", () => {
    render(
      <Button asChild variant="outline">
        <a href="https://example.com">
          <ExternalLink />
          Open docs
        </a>
      </Button>,
    );

    const link = screen.getByRole("link", { name: "Open docs" });
    const label = link.children.item(0);

    expect(link).not.toHaveClass(styles.iconOnly);
    expect(link.children).toHaveLength(1);
    expect(label).toHaveTextContent("Open docs");
    expect(label?.querySelector("svg")).toHaveClass("lucide-external-link");
  });

  it("prevents disabled asChild links from handling clicks", async () => {
    const onChildClick = vi.fn();
    const onButtonClick = vi.fn();

    const { user } = render(
      <Button asChild disabled onClick={onButtonClick}>
        <a href="https://example.com" onClick={onChildClick}>
          Disabled link
        </a>
      </Button>,
    );

    const link = screen.getByRole("link", { name: "Disabled link" });

    await user.click(link);

    expect(link).toHaveAttribute("aria-disabled", "true");
    expect(link).toHaveAttribute("tabindex", "-1");
    expect(link).not.toHaveAttribute("disabled");
    expect(onChildClick).not.toHaveBeenCalled();
    expect(onButtonClick).not.toHaveBeenCalled();
  });
});

describe("IconButton", () => {
  it("keeps icon-only button behavior", () => {
    render(<IconButton aria-label="Add item" icon={Plus} />);

    const button = screen.getByRole("button", { name: "Add item" });

    expect(button).toBeInTheDocument();
    expect(button).toHaveAttribute("type", "button");
    expect(button).toHaveClass("rf-pressable");
  });
});
