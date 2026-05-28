import { describe, expect, it } from "vitest";
import { extractCodeLines } from "./editToolHighlight";

describe("extractCodeLines", () => {
  it("extracts full highlighted lines with nested Shiki spans", () => {
    const html = `<pre><code><span class="line highlighted" data-line="1"><span style="color:#79c0ff">expect</span><span>(state.container?.textContent).</span><span>toContain</span><span>("Planning next steps")</span></span></code></pre>`;

    expect(extractCodeLines(html)).toEqual([
      '<span style="color:#79c0ff">expect</span><span>(state.container?.textContent).</span><span>toContain</span><span>("Planning next steps")</span>',
    ]);
  });
});
