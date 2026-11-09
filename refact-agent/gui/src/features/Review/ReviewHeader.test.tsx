import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ReviewHeader } from "./ReviewHeader";

describe("ReviewHeader", () => {
  it("renders identity and all supplied controls in one header", () => {
    render(
      <ReviewHeader
        identity={<span>branch</span>}
        stackControls={<button>Stack</button>}
        reviewControls={<button>Review</button>}
        status={<span title="metadata">Browsing</span>}
      />,
    );
    const header = screen.getByRole("banner");
    expect(header).toContainElement(
      screen.getByRole("button", { name: "Stack" }),
    );
    expect(header).toContainElement(
      screen.getByRole("button", { name: "Review" }),
    );
    expect(screen.getAllByText("branch")).toHaveLength(1);
    expect(screen.getByText("Browsing")).toHaveAttribute("title", "metadata");
  });
});
