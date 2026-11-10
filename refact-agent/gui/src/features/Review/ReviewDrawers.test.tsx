import { useState } from "react";
import { screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { render } from "../../utils/test-utils";
import { Drawer, SourcesAndEvidence } from "./ReviewDrawers";
import { anchor, makeReview } from "./reviewFixtures";

describe("Drawer disclosure", () => {
  it("retains nested input state while closed", async () => {
    function Example() {
      const [open, setOpen] = useState(true);
      return (
        <Drawer title="Workers" open={open} onToggle={() => setOpen(!open)}>
          <label>
            Draft
            <input />
          </label>
        </Drawer>
      );
    }
    const { user } = render(<Example />);
    await user.type(
      screen.getByRole("textbox", { name: "Draft" }),
      "unfinished",
    );
    const toggle = screen.getByRole("button", { name: "Workers" });
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(
      screen.queryByRole("textbox", { name: "Draft" }),
    ).not.toBeInTheDocument();
    await user.click(toggle);
    expect(screen.getByRole("textbox", { name: "Draft" })).toHaveValue(
      "unfinished",
    );
  });
});

it("presents evidence rationale and exact source context without inventing execution", async () => {
  const source = {
    id: "source",
    candidate_id: "c1",
    material_id: null,
    run_id: null,
    source: {
      kind: "source" as const,
      path: "src/a.ts",
      anchor: anchor("c1", "src/a.ts", "head", 3),
    },
    summary: "This branch preserves the retry guard.",
    applicability: "current" as const,
    created_at_ms: 1,
  };
  const reveal = vi.fn();
  const { user } = render(
    <SourcesAndEvidence
      review={makeReview({ evidence: [source] })}
      candidateId="c1"
      selectedAnnotationId={null}
      onRevealSource={reveal}
    />,
  );
  const card = screen.getByRole("article", { name: /Evidence: This branch/ });
  expect(card).toHaveTextContent("Head L3");
  expect(card).toHaveTextContent("Candidate c1");
  expect(card).not.toHaveTextContent("exit");
  await user.click(
    within(card).getByRole("button", { name: "Show range in diff" }),
  );
  expect(reveal).toHaveBeenCalledWith(source);
});

it("distinguishes file-only context from executed command results", () => {
  const common = {
    candidate_id: "c1",
    material_id: null,
    run_id: null,
    applicability: "current" as const,
    created_at_ms: 1,
  };
  const review = makeReview({
    evidence: [
      {
        ...common,
        id: "file",
        summary: "File context",
        source: { kind: "source", path: "src/a.ts", anchor: null },
      },
      {
        ...common,
        id: "run",
        summary: "Tests passed",
        source: {
          kind: "executed",
          command: ["npm", "test"],
          workdir: "/repo",
          exit_code: 0,
          stdout: null,
          stderr: null,
        },
      },
    ],
  });
  render(
    <SourcesAndEvidence
      review={review}
      candidateId="c1"
      selectedAnnotationId={null}
      onRevealSource={vi.fn()}
    />,
  );
  expect(
    screen.getByRole("button", { name: /file context only/ }),
  ).toBeVisible();
  expect(
    screen.getByRole("article", { name: "Evidence: Tests passed" }),
  ).toHaveTextContent("exit 0");
});
