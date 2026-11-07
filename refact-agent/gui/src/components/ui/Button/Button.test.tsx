import { ChevronRight, ExternalLink, Plus } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { Button, IconButton } from "./Button";

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

  it("renders rightIcon after the label without extra placeholders", () => {
    render(<Button rightIcon={ChevronRight}>Next</Button>);

    const button = screen.getByRole("button", { name: "Next" });
    const icon = button.children.item(1);

    expect(icon).toHaveAttribute("aria-hidden", "true");
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

    expect(button.children).toHaveLength(1);
    expect(icon).toHaveAttribute("aria-hidden", "true");
    expect(icon).not.toHaveTextContent("Add item");
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
