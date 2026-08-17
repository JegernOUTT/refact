import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { DesignFrame } from "./DesignFrame";
import {
  DEFAULT_DESIGN_FALLBACK_REASON,
  HOST_ORIGIN_FALLBACK_REASON,
  resolveDesignRenderer,
} from "./designRenderer";
import { makeDesignSurfaceState } from "./designSlice";

const handlers = {
  onBasicLoad: () => undefined,
  onBlocked: () => undefined,
  onMessage: () => undefined,
};

describe("DesignFrame", () => {
  it("selects each renderer in the fallback ladder", () => {
    const state = makeDesignSurfaceState();

    expect(resolveDesignRenderer(state)).toBe("empty");
    expect(
      resolveDesignRenderer({
        ...state,
        liveUrl: "http://localhost:5173",
        liveStatus: "interactive",
      }),
    ).toBe("live-interactive");
    expect(
      resolveDesignRenderer({
        ...state,
        liveUrl: "http://localhost:5173",
        liveStatus: "basic",
      }),
    ).toBe("live-basic");
    expect(
      resolveDesignRenderer({
        ...state,
        liveUrl: "https://blocked.example",
        liveStatus: "blocked",
      }),
    ).toBe("browser-frame");
    expect(resolveDesignRenderer({ ...state, source: "artifact" })).toBe(
      "artifact",
    );
    expect(resolveDesignRenderer({ ...state, source: "reference" })).toBe(
      "reference",
    );
  });

  it("renders a sandboxed artifact srcDoc", () => {
    const state = {
      ...makeDesignSurfaceState(),
      source: "artifact" as const,
      artifactHtml: "<button>Preview</button>",
    };

    render(<DesignFrame {...handlers} browserFrame={null} state={state} />);

    const frame = screen.getByTitle("Design artifact preview");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame).toHaveAttribute("referrerpolicy", "no-referrer");
    expect(frame).toHaveAttribute("srcdoc", expect.stringContaining("Preview"));
  });

  it("renders the reference image beside the live preview", () => {
    const state = {
      ...makeDesignSurfaceState(),
      source: "reference" as const,
      referenceDataUrl: "data:image/png;base64,reference",
    };

    render(<DesignFrame {...handlers} browserFrame={null} state={state} />);

    expect(screen.getByAltText("Design reference")).toHaveAttribute(
      "src",
      state.referenceDataUrl,
    );
  });

  it("renders a CDP frame and visible reason when iframe embedding is blocked", () => {
    const state = {
      ...makeDesignSurfaceState(),
      liveUrl: "https://blocked.example",
      liveStatus: "blocked" as const,
      fallbackReason: "The host sent X-Frame-Options: DENY.",
    };

    render(
      <DesignFrame
        {...handlers}
        browserFrame={{
          mime: "image/png",
          data: "frame",
          diff_boxes: [],
        }}
        state={state}
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent(
      "The host sent X-Frame-Options: DENY.",
    );
    expect(screen.getByAltText("CDP browser fallback frame")).toHaveAttribute(
      "src",
      "data:image/png;base64,frame",
    );
  });

  it("never leaves a blocked fallback reason blank", () => {
    const state = {
      ...makeDesignSurfaceState(),
      liveUrl: "https://blocked.example",
      liveStatus: "blocked" as const,
      fallbackReason: null,
    };

    render(<DesignFrame {...handlers} browserFrame={null} state={state} />);

    expect(screen.getByRole("status")).toHaveTextContent(
      DEFAULT_DESIGN_FALLBACK_REASON,
    );
  });

  it("does not embed same-origin content that could access host APIs", () => {
    const state = {
      ...makeDesignSurfaceState(),
      liveUrl: `${window.location.origin}/preview`,
      liveStatus: "basic" as const,
    };

    render(<DesignFrame {...handlers} browserFrame={null} state={state} />);

    expect(screen.getByRole("status")).toHaveTextContent(
      HOST_ORIGIN_FALLBACK_REASON,
    );
    expect(screen.queryByTitle("Live design preview")).not.toBeInTheDocument();
  });
});
