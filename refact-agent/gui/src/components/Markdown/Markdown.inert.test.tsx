import { describe, expect, it } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import { render } from "../../utils/test-utils";
import { Markdown } from "./Markdown";

const image = "![chart](https://host.example/c.png?d=1)";
const htmlFence = "```html\n<p>hello</p>\n```";
const svgFence = "```svg\n<svg xmlns='http://www.w3.org/2000/svg'></svg>\n```";
const puzzle = "🧩SETTINGS:editor:/etc/hosts";

describe("Markdown default (unchanged for existing callers)", () => {
  it("loads an image", () => {
    const { container } = render(<Markdown>{image}</Markdown>);
    expect(container.querySelector("img")).toHaveAttribute(
      "src",
      "https://host.example/c.png?d=1",
    );
  });
  it("mounts an html fence as an artifact iframe", async () => {
    const { container } = render(<Markdown>{htmlFence}</Markdown>);
    await waitFor(() =>
      expect(container.querySelector("iframe")).not.toBeNull(),
    );
  });
  it("turns a puzzle line into an active link button", () => {
    render(<Markdown canHaveInteractiveElements>{puzzle}</Markdown>);
    expect(screen.getByRole("button")).toBeInTheDocument();
  });
});

describe("Markdown inert (untrusted script text)", () => {
  it("never loads an image and shows its alt text and URL", () => {
    const { container } = render(<Markdown inert>{image}</Markdown>);
    expect(container.querySelector("img")).toBeNull();
    expect(
      screen.getByText("chart (https://host.example/c.png?d=1)"),
    ).toBeInTheDocument();
  });
  it("never mounts an html fence as an iframe and shows the code", async () => {
    const { container } = render(<Markdown inert>{htmlFence}</Markdown>);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(container.querySelector("iframe")).toBeNull();
    expect(container.textContent).toContain("<p>hello</p>");
  });
  it("shows an svg fence as code, not a diagram", () => {
    const { container } = render(<Markdown inert>{svgFence}</Markdown>);
    expect(container.querySelector("svg[xmlns]")).toBeNull();
    expect(container.textContent).toContain("<svg xmlns=");
  });
  it("never activates a puzzle link, even when interactive elements are asked for", () => {
    const { container } = render(
      <Markdown inert canHaveInteractiveElements>
        {puzzle}
      </Markdown>,
    );
    expect(screen.queryByRole("button")).toBeNull();
    expect(container.textContent).toContain(puzzle);
  });
});
