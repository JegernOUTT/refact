import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppIcon } from "./AppIcon";
import type { AppIconArtwork } from "../../services/refact/apps";

describe("AppIcon", () => {
  it("renders bundled geometry without markup or network resources", () => {
    const artwork: AppIconArtwork = {
      view_box: [0, 0, 24, 24],
      nodes: [
        {
          tag: "g",
          attrs: { fill: "none", stroke: "currentColor" },
          children: [
            { tag: "path", attrs: { d: "M2 3L12 20 22 3Z" }, children: [] },
            {
              tag: "circle",
              attrs: { cx: "12", cy: "9", r: "2" },
              children: [],
            },
          ],
        },
      ],
    };
    const { container } = render(
      <AppIcon icon="assets/icon.svg" artwork={artwork} size={28} />,
    );
    expect(container.querySelector("svg")?.getAttribute("viewBox")).toBe(
      "0 0 24 24",
    );
    expect(container.querySelector("path")?.getAttribute("d")).toBe(
      "M2 3L12 20 22 3Z",
    );
    expect(container.querySelector("circle")?.getAttribute("r")).toBe("2");
    expect(container.querySelector("svg")?.getAttribute("aria-hidden")).toBe(
      "true",
    );
    expect(
      container.querySelector(
        "img,script,image,use,foreignObject,[href],[src]",
      ),
    ).toBeNull();
  });

  it("falls back instead of rendering a malformed server artwork descriptor", () => {
    // Simulate a compromised/old response at the runtime boundary, not a TS caller.
    const artwork = JSON.parse(
      '{"view_box":[0,0,24,24],"nodes":[{"tag":"path","attrs":{"d":"M1 2L3 4","onclick":"alert(1)","fill":"url(https://evil.test)"},"children":[]}]}',
    ) as AppIconArtwork;
    const { container } = render(
      <AppIcon icon="lucide:timer" artwork={artwork} />,
    );
    expect(container.querySelector(".lucide-timer")).not.toBeNull();
    expect(container.querySelector("[onclick],[href],[src]")).toBeNull();
    expect(container.innerHTML).not.toContain("evil.test");
    expect(container.innerHTML).not.toContain("M1 2L3 4");
  });
});
