import { fireEvent, render, screen, within } from "@testing-library/react";
import { createPortal } from "react-dom";
import { describe, expect, it, vi } from "vitest";
import { ReviewHeader } from "./ReviewHeader";
import { ReviewCanvas } from "./ReviewCanvas";
import { render as renderWithProviders } from "../../utils/test-utils";
import { makeReview } from "./reviewFixtures";

vi.mock("./ReviewFileDiff", () => ({
  ReviewFileDiff: ({ path }: { path: string }) => (
    <div data-testid="selected-file">{path}</div>
  ),
}));

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
    expect(
      within(header)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Stack", "Review"]);
    expect(header.textContent).toBe("StackbranchBrowsingReview");
    expect(header).toContainElement(
      screen.getByRole("button", { name: "Review" }),
    );
    expect(screen.getAllByText("branch")).toHaveLength(1);
    expect(screen.getByText("Browsing")).toHaveAttribute("title", "metadata");
  });

  it("scrolls its single row for its own controls but not for portalled content", () => {
    render(
      <ReviewHeader
        identity={<span>branch</span>}
        reviewControls={
          <>
            <button>Inside</button>
            {createPortal(<button>Portalled</button>, document.body)}
          </>
        }
      />,
    );
    const header = screen.getByRole("banner");
    const inside = screen.getByRole("button", { name: "Inside" });
    const portalled = screen.getByRole("button", { name: "Portalled" });
    header.style.overflowX = "auto";
    expect(getComputedStyle(header).overflowX).toBe("auto");
    const rect = (left: number, right: number): DOMRect => ({
      left,
      right,
      top: 0,
      bottom: 20,
      width: right - left,
      height: 20,
      x: left,
      y: 0,
      toJSON: () => ({}),
    });
    vi.spyOn(header, "getBoundingClientRect").mockReturnValue(rect(0, 100));
    vi.spyOn(inside, "getBoundingClientRect").mockReturnValue(rect(120, 160));
    vi.spyOn(portalled, "getBoundingClientRect").mockReturnValue(
      rect(500, 600),
    );
    fireEvent.wheel(portalled, { deltaY: 80 });
    portalled.focus();
    expect(header.scrollLeft).toBe(0);
    fireEvent.wheel(inside, { deltaY: 80 });
    expect(header.scrollLeft).toBe(80);
    inside.focus();
    expect(header.scrollLeft).toBe(140);
  });
});

describe("Review header file visibility controls", () => {
  it("selects the first changed file when hiding the selected unchanged file", async () => {
    const { user } = renderWithProviders(
      <ReviewCanvas review={makeReview()} />,
    );
    await user.click(screen.getByRole("radio", { name: "Review" }));
    await user.click(
      screen.getByRole("button", { name: "More review options" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Show unchanged files" }),
    );
    await user.click(
      within(screen.getByRole("list", { name: "Unchanged files" })).getByRole(
        "button",
        { name: "src/same.ts" },
      ),
    );
    expect(document.querySelector('[class*="_filePath_"]')).toHaveTextContent(
      "src/same.ts",
    );
    await user.click(
      screen.getByRole("button", { name: "More review options" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Hide unchanged files" }),
    );
    expect(
      within(screen.getByRole("list", { name: "Changed files" })).getByRole(
        "button",
        { name: "src/a.ts" },
      ),
    ).toHaveAttribute("aria-current", "true");
    expect(document.querySelector('[class*="_filePath_"]')).toHaveTextContent(
      "src/a.ts",
    );
  });

  it("shows an empty unchanged-files item", async () => {
    const review = makeReview();
    review.candidates[0].files = review.candidates[0].files.slice(0, 1);
    const { user } = renderWithProviders(<ReviewCanvas review={review} />);
    await user.click(screen.getByRole("radio", { name: "Review" }));
    await user.click(
      screen.getByRole("button", { name: "More review options" }),
    );
    await user.click(
      screen.getByRole("menuitem", { name: "Show unchanged files" }),
    );
    expect(
      within(screen.getByRole("list", { name: "Unchanged files" })).getByRole(
        "listitem",
      ),
    ).toHaveTextContent("No unchanged files.");
  });
});
