import { describe, it, expect } from "vitest";
import { render, screen } from "../../utils/test-utils";
import { BuddyArtifactBody } from "./BuddyArtifactBody";

describe("Buddy artifact body", () => {
  it("fences machine payload lines into a code block", () => {
    const { container } = render(
      <BuddyArtifactBody
        body={
          "### Retained source signal\nmemory_pulse total=22184 orphan=5\nmemory_orphan key=memory:orphan:batch:79a9 count=5\n\nPlain closing sentence."
        }
      />,
    );
    expect(container.querySelector("pre")?.textContent).toContain(
      "memory_pulse total=22184",
    );
    expect(screen.getByText("Plain closing sentence.").tagName).toBe("P");
  });

  it("keeps chat inline-code typography but renders report links inert", () => {
    const { container } = render(
      <BuddyArtifactBody
        body={"Use `npm test` and see [docs](https://example.com)."}
      />,
    );
    expect(container.querySelector("code")?.className).toMatch(/code_inline/);
    expect(screen.getByText("docs").tagName).toBe("SPAN");
    expect(container.querySelector("a[href]")).toBeNull();
  });

  it("demotes Markdown headings while preserving their text and order", () => {
    const { container } = render(
      <BuddyArtifactBody
        body={
          "# Stored H1\n\n## Stored H2\n\n### Stored H3\n\n#### Stored H4\n\n##### Stored H5\n\n###### Stored H6"
        }
      />,
    );
    const headings = [...container.querySelectorAll("h1, h2, h3, h4, h5, h6")];
    expect(headings.map((heading) => heading.tagName)).toEqual([
      "H4",
      "H4",
      "H4",
      "H4",
      "H4",
      "H4",
    ]);
    expect(headings.map((heading) => heading.textContent)).toEqual([
      "Stored H1",
      "Stored H2",
      "Stored H3",
      "Stored H4",
      "Stored H5",
      "Stored H6",
    ]);
    expect(
      headings.every((heading) => Number(heading.tagName.slice(1)) >= 4),
    ).toBe(true);
  });

  it("renders readable reports without allowing embedded actions, images or executable HTML", () => {
    const { container } = render(
      <BuddyArtifactBody
        body={
          "## Findings\n- **Authentication** failed\n\n[External](https://external.invalid/collect)\n\n[Execute](refact://run/action)\n\n![private](https://external.invalid/track)\n\n<script>alert(1)</script>\n\n```html\n<button>apply</button>\n```"
        }
      />,
    );
    expect(screen.getByRole("heading", { name: "Findings" })).toBeTruthy();
    expect(screen.getByText("Authentication")).toBeTruthy();
    expect(screen.getByText("Image omitted: private")).toBeTruthy();
    expect(container.querySelector("a,img,script,iframe,button")).toBeNull();
    expect(container.querySelector("img[src]")).toBeNull();
    expect(container.querySelector("pre")?.textContent).toContain(
      "<button>apply</button>",
    );
  });
});
