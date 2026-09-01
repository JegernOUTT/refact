import { describe, expect, it } from "vitest";
import { render } from "../../utils/test-utils";
import { Markdown } from "./Markdown";

describe("Markdown math", () => {
  it.each([
    ["inline dollars", "$x$", false],
    ["display dollars", "$$\nx^2\n$$", true],
    ["inline LaTeX", "\\(z_{\\mathrm{detail}}\\)", false],
    ["display LaTeX", "\\[\na_m=1-\\cos(p_m,\\bar p)\n\\]", true],
  ])("renders %s with KaTeX", (_name, markdown, isDisplay) => {
    const { container } = render(<Markdown>{markdown}</Markdown>);

    expect(container.querySelector(".katex")).not.toBeNull();
    if (isDisplay) {
      expect(container.querySelector(".katex-display")).not.toBeNull();
    } else {
      expect(container.querySelector(".katex-display")).toBeNull();
    }
  });

  it("renders display LaTeX that is not at the start of the document", () => {
    const { container } = render(
      <Markdown>{"some text \\[ x^2 \\] more text"}</Markdown>,
    );

    expect(container.querySelector(".katex")).not.toBeNull();
    expect(container.querySelector(".katex-display")).not.toBeNull();
    expect(container.textContent).toContain("some text");
    expect(container.textContent).toContain("more text");
  });

  it.each([
    ["inline code", "`\\(x\\)`", "\\(x\\)"],
    [
      "multi-backtick inline code",
      "``\\[y\\] with ` inside``",
      "\\[y\\] with ` inside",
    ],
    ["backtick fence", "```latex\n\\(x\\)\n\\[y\\]\n```", "\\(x\\)\n\\[y\\]"],
    ["tilde fence", "~~~latex\n\\(x\\)\n\\[y\\]\n~~~", "\\(x\\)\n\\[y\\]"],
  ])("leaves delimiters inside %s literal", (_name, markdown, literal) => {
    const { container } = render(<Markdown>{markdown}</Markdown>);

    expect(container.querySelector(".katex")).toBeNull();
    expect(container.textContent).toContain(literal);
  });
});
