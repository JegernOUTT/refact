import { screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { render } from "../../utils/test-utils";
import { ReviewDocument } from "./ReviewDocument";
import { ReviewDocumentState, RichDiscussionContext } from "./ReviewRichBlocks";
import { makeReview } from "./reviewFixtures";

const topic = (id: string, title: string) => ({
  id,
  current: { version: 1, title, body: `Body for ${title}`, anchors: [] },
  history: [],
});

function documentView(review: ReturnType<typeof makeReview>) {
  return (
    <ReviewDocumentState>
      <ReviewDocument
        review={review}
        stepId="alpha"
        active
        onManualScroll={vi.fn()}
        onRevealAnchor={vi.fn()}
      />
    </ReviewDocumentState>
  );
}

describe("living review document", () => {
  it("offers every historical version through the shared accessible selector", async () => {
    const original = topic("alpha", "Cache lifecycle");
    const review = makeReview({
      tour: [
        {
          ...original,
          current: { ...original.current, version: 3, body: "Latest body" },
          history: [
            { ...original.current, version: 2, body: "Intermediate body" },
            original.current,
          ],
        },
      ],
    });
    const { user } = render(documentView(review));
    screen
      .getByRole("combobox", { name: "Version of Cache lifecycle" })
      .focus();
    await user.keyboard("{Enter}");
    expect(
      screen.getByRole("option", { name: "v3 · latest" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: "v2 · historical" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: "v1 · historical" }));
    expect(screen.getByText("Body for Cache lifecycle")).toBeInTheDocument();
    expect(screen.queryByText("Latest body")).not.toBeInTheDocument();
    expect(
      screen.getByRole("combobox", { name: "Version of Cache lifecycle" }),
    ).toHaveFocus();
  });

  it("reveals historical canonical threads outside the collapsed composer", () => {
    const old = topic("alpha", "Original");
    const target = { candidate_id: "c1", step_id: "alpha", version: 1 };
    const review = makeReview({
      tour: [
        {
          ...old,
          current: { ...old.current, version: 2 },
          history: [old.current],
        },
      ],
      threads: [
        {
          id: "historical-thread",
          target,
          anchors: [],
          annotation_id: null,
          resolved: false,
          contributions: [],
        },
      ],
    });
    render(
      <ReviewDocumentState>
        <RichDiscussionContext.Provider value={(id) => <p>Canonical {id}</p>}>
          <ReviewDocument
            review={review}
            stepId="alpha"
            active
            onManualScroll={vi.fn()}
            onRevealAnchor={vi.fn()}
            revealTarget={{ target, token: 1, threadId: "historical-thread" }}
          />
        </RichDiscussionContext.Provider>
      </ReviewDocumentState>,
    );
    expect(screen.getByText("Canonical historical-thread")).toBeVisible();
    expect(
      screen.queryByRole("textbox", { name: "Question about this block" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Topic history" }).closest("header"),
    ).not.toBeNull();
  });

  it("shows one page at a time and pages through the tour by callback", async () => {
    const review = makeReview({
      tour: [
        topic("alpha", "Cache lifecycle"),
        topic("beta", "Failure recovery"),
      ],
    });
    const onSelectStep = vi.fn();
    const view = (stepId: string | null) => (
      <ReviewDocumentState>
        <ReviewDocument
          review={review}
          stepId={stepId}
          active
          onSelectStep={onSelectStep}
          onManualScroll={vi.fn()}
          onRevealAnchor={vi.fn()}
        />
      </ReviewDocumentState>
    );
    const { rerender, user } = render(view(null));
    expect(
      screen.getByRole("heading", { name: "Cache lifecycle" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Failure recovery" }),
    ).not.toBeInTheDocument();
    const pager = screen.getByRole("navigation", { name: "Tour pages" });
    expect(within(pager).getByText("Page 1 of 2")).toBeInTheDocument();
    expect(
      within(pager).getByRole("button", { name: "Previous" }),
    ).toBeDisabled();
    await user.click(
      within(pager).getByRole("button", { name: "Failure recovery" }),
    );
    expect(onSelectStep).toHaveBeenCalledWith("beta");
    rerender(view("beta"));
    expect(
      screen.getByRole("heading", { name: "Failure recovery" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Page 2 of 2")).toBeInTheDocument();
  });

  it("says the tour is still being written instead of showing an empty page", () => {
    render(
      <ReviewDocumentState>
        <ReviewDocument
          review={makeReview({ tour: [] })}
          stepId={null}
          active
          working
          onManualScroll={vi.fn()}
          onRevealAnchor={vi.fn()}
        />
      </ReviewDocumentState>,
    );
    expect(
      screen.getByRole("heading", { name: "The tour is being written" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("navigation", { name: "Tour pages" }),
    ).not.toBeInTheDocument();
  });

  it("offers a newer version rather than replacing the version being read", async () => {
    const original = topic("alpha", "Cache lifecycle");
    const review = makeReview({ tour: [original] });
    const { rerender, user } = render(documentView(review));
    const newer = {
      ...original,
      current: {
        ...original.current,
        version: 2,
        title: "Updated cache lifecycle",
        body: "New body",
      },
      history: [original.current],
    };
    rerender(documentView({ ...review, tour: [newer] }));
    expect(
      screen.getByRole("heading", { name: "Cache lifecycle" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("New body")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: /Apply updated topic/ }),
    );
    expect(
      screen.getByRole("heading", { name: "Updated cache lifecycle" }),
    ).toBeInTheDocument();
    expect(screen.getByText("New body")).toBeInTheDocument();
  });
});
