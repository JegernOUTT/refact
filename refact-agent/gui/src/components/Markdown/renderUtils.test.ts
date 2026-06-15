import { describe, expect, test } from "vitest";
import {
  clampPan,
  makeCrispSvg,
  maskIncompleteSpecialCodeFences,
  parseSvgMeta,
  stripExternalRefs,
  wrapArtifactHtml,
} from "./renderUtils";

describe("maskIncompleteSpecialCodeFences", () => {
  test("masks an unterminated mermaid fence to a safe language", () => {
    const input = "```mermaid\nflowchart LR\nA --> B";
    expect(maskIncompleteSpecialCodeFences(input)).toBe(
      "```text\nflowchart LR\nA --> B",
    );
  });

  test("masks unterminated html and svg fences", () => {
    expect(maskIncompleteSpecialCodeFences("```html\n<div>")).toBe(
      "```text\n<div>",
    );
    expect(maskIncompleteSpecialCodeFences("```svg\n<svg>")).toBe(
      "```text\n<svg>",
    );
  });

  test("masks uppercase fence info strings", () => {
    expect(maskIncompleteSpecialCodeFences("```MERMAID\nflowchart")).toBe(
      "```text\nflowchart",
    );
  });

  test("masks tilde fences", () => {
    expect(maskIncompleteSpecialCodeFences("~~~html\n<div>")).toBe(
      "~~~text\n<div>",
    );
  });

  test("leaves closed special fences untouched", () => {
    const input = "```mermaid\nflowchart LR\nA --> B\n```";
    expect(maskIncompleteSpecialCodeFences(input)).toBe(input);
  });

  test("leaves non-special unterminated fences untouched", () => {
    const input = "```python\nprint('hi')";
    expect(maskIncompleteSpecialCodeFences(input)).toBe(input);
  });

  test("ignores special fences that already closed before regular text", () => {
    const input = "```html\n<div></div>\n```\nplain text after";
    expect(maskIncompleteSpecialCodeFences(input)).toBe(input);
  });
});

describe("makeCrispSvg", () => {
  test("rewrites sizing on the root svg only, keeping child attributes", () => {
    const input =
      '<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" style="border:1px" viewBox="0 0 100 50">' +
      '<rect width="10" height="5" style="fill:red"/></svg>';
    const out = makeCrispSvg(input, "0 0 100 50");

    expect(out).toMatch(/<svg[^>]*width="100%"/);
    expect(out).toMatch(/<svg[^>]*height="100%"/);
    expect(out).toMatch(/<svg[^>]*viewBox="0 0 100 50"/);
    expect(out).not.toMatch(/<svg[^>]*style=/);

    expect(out).toMatch(/<rect[^>]*width="10"/);
    expect(out).toMatch(/<rect[^>]*height="5"/);
    expect(out).toMatch(/<rect[^>]*style="fill:red"/);
  });

  test("returns input unchanged when the root element is not svg", () => {
    const input = "<div>not an svg</div>";
    expect(makeCrispSvg(input, "0 0 1 1")).toBe(input);
  });
});

describe("parseSvgMeta", () => {
  test("derives dimensions from absolute width/height attributes", () => {
    const meta = parseSvgMeta('<svg width="120" height="60"></svg>');
    expect(meta).not.toBeNull();
    expect(meta?.width).toBe(120);
    expect(meta?.height).toBe(60);
    expect(meta?.viewBox).toBe("0 0 120 60");
  });

  test("returns null when no usable dimensions exist", () => {
    expect(parseSvgMeta('<svg width="100%" height="100%"></svg>')).toBeNull();
  });

  test("returns null for non-svg content", () => {
    expect(parseSvgMeta("<div></div>")).toBeNull();
  });
});

describe("clampPan", () => {
  test("keeps part of the content visible when panning far left/up", () => {
    expect(clampPan(-5000, 500, 1000)).toBe(48 - 1000);
  });

  test("keeps part of the content visible when panning far right/down", () => {
    expect(clampPan(5000, 500, 1000)).toBe(500 - 48);
  });

  test("passes through in-range pan values", () => {
    expect(clampPan(10, 500, 1000)).toBe(10);
  });

  test("degrades gracefully when the canvas is smaller than the margin", () => {
    expect(clampPan(5000, 30, 1000)).toBe(0);
    expect(clampPan(-5000, 30, 1000)).toBe(30 - 1000);
  });
});

describe("stripExternalRefs", () => {
  test("removes external href values but keeps internal and data refs", () => {
    const input =
      '<svg xmlns="http://www.w3.org/2000/svg">' +
      '<use href="#icon"/>' +
      '<image href="https://example.com/x.png"/>' +
      '<image href="data:image/png;base64,AAA"/>' +
      "</svg>";
    const out = stripExternalRefs(input);

    expect(out).toContain('href="#icon"');
    expect(out).toContain('href="data:image/png;base64,AAA"');
    expect(out).not.toContain("example.com");
  });

  test("removes external url(...) references in paint and filter attributes", () => {
    const input =
      '<svg xmlns="http://www.w3.org/2000/svg">' +
      '<rect fill="url(https://evil.example/p.svg#x)" stroke="url(#grad)"/>' +
      '<circle filter="url(//evil.example/f)"/>' +
      '<path clip-path="url(#clip)" mask="url(&quot;https://evil.example&quot;)"/>' +
      "</svg>";
    const out = stripExternalRefs(input);

    expect(out).not.toContain("evil.example");
    expect(out).toContain('stroke="url(#grad)"');
    expect(out).toContain('clip-path="url(#clip)"');
  });

  test("removes external url(...) references in style attributes", () => {
    const input =
      '<svg xmlns="http://www.w3.org/2000/svg">' +
      "<rect style=\"fill:url('https://evil.example/x')\"/>" +
      '<circle style="fill:url(#ok)"/>' +
      "</svg>";
    const out = stripExternalRefs(input);

    expect(out).not.toContain("evil.example");
    expect(out).toContain("url(#ok)");
  });
});

describe("wrapArtifactHtml", () => {
  test("wraps snippets into a complete themable document", () => {
    const out = wrapArtifactHtml("<div>hi</div>");

    expect(out.startsWith("<!DOCTYPE html>")).toBe(true);
    expect(out).toContain("<div>hi</div>");
    expect(out).toContain("refact-artifact-resize");
    expect(out).toContain("refact-artifact-theme");
    expect(out).toContain(":where(html:not([data-artifact-styled]) body)");
    // Theme must arrive via postMessage, never baked into the markup, so
    // appearance flips do not rewrite srcDoc and reload the iframe.
    expect(out).not.toMatch(/<html[^>]+data-refact-theme/);
  });

  test("injects defaults before author styles in complete documents", () => {
    const doc =
      "<!DOCTYPE html><html><head><style>body{background:red}</style></head>" +
      "<body><p>x</p></body></html>";
    const out = wrapArtifactHtml(doc);

    const injected = out.indexOf("data-refact-artifact");
    const author = out.indexOf("background:red");
    expect(injected).toBeGreaterThan(-1);
    expect(author).toBeGreaterThan(-1);
    expect(injected).toBeLessThan(author);
    expect(out).toContain("<script data-refact-artifact");
  });

  test("installs error handlers before author scripts run", () => {
    const doc =
      "<!DOCTYPE html><html><head></head>" +
      "<body><script>throw new Error('boom')</script></body></html>";
    const out = wrapArtifactHtml(doc);

    const bootstrap = out.indexOf("window.onerror");
    const author = out.indexOf("throw new Error('boom')");
    expect(bootstrap).toBeGreaterThan(-1);
    expect(author).toBeGreaterThan(-1);
    expect(bootstrap).toBeLessThan(author);
  });

  test("installs error handlers before snippet content", () => {
    const out = wrapArtifactHtml("<script>boom()</script>");

    expect(out.indexOf("window.onerror")).toBeLessThan(out.indexOf("boom()"));
  });

  test("does not mistake <header> for <head>", () => {
    const doc =
      "<!DOCTYPE html><html><body><header>h</header><p>x</p></body></html>";
    const out = wrapArtifactHtml(doc);

    expect(out.indexOf("data-refact-artifact")).toBeLessThan(
      out.indexOf("<header>"),
    );
  });

  test("appends scripts when the document has no closing body tag", () => {
    const doc = "<html><body><p>x</p>";
    const out = wrapArtifactHtml(doc);
    expect(out).toContain("refact-artifact-resize");
  });
});
