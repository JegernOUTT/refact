import { describe, expect, it } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { WorkToolView } from "./WorkToolView";
import { workFixture } from "../../../services/refact/workFixtures";

describe("Work tool card", () => {
  it("renders structured states rather than raw JSON", () => {
    render(
      <WorkToolView
        content={JSON.stringify(workFixture)}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText("Board work")).toBeInTheDocument();
    expect(screen.getByText("Queued")).toBeInTheDocument();
    expect(screen.queryByText(/snapshot_id/)).not.toBeInTheDocument();
  });
  it("supports streaming and unavailable legacy/error results", () => {
    const view = render(<WorkToolView content={null} pending failed={false} />);
    expect(screen.getByRole("status")).toHaveTextContent("Waiting");
    view.rerender(
      <WorkToolView content="legacy response" pending={false} failed={false} />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("legacy");
  });

  it("shows the inspected item independently of its children page", () => {
    render(
      <WorkToolView
        content={JSON.stringify({
          ...workFixture,
          row: workFixture.rows[0],
          rows: [],
          total: 0,
          returned: 0,
        })}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByLabelText("Selected work")).toHaveTextContent(
      "Board work",
    );
    expect(screen.queryByText(/No work observed/)).not.toBeInTheDocument();
  });
});

const peerRow = {
  ...workFixture.rows[0],
  key: { runtime: "trajectory", id: "peer" },
  title: "Peer work",
  root: { ...workFixture.rows[0].root, id: "other", branch: "feature/peer" },
};

function inspection(extra: Record<string, unknown>) {
  return JSON.stringify({
    ...workFixture,
    selector: { kind: "files", paths: ["src/lib.rs"] },
    rows: [workFixture.rows[0], peerRow],
    total: 2,
    returned: 2,
    ...extra,
  });
}

describe("Work tool card file overlap", () => {
  it("renders nothing when the inspection reports no overlapping work", () => {
    render(
      <WorkToolView
        content={inspection({ rows: [], total: 0, returned: 0 })}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.queryByLabelText("File overlap")).not.toBeInTheDocument();
    expect(screen.queryByText(/same physical file/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/separate checkout/i)).not.toBeInTheDocument();
  });

  it("separates a physical file conflict from an advisory cross-checkout overlap", () => {
    render(
      <WorkToolView
        content={inspection({ integration_overlaps: ["trajectory:peer"] })}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByLabelText("File overlap")).toBeInTheDocument();
    expect(screen.getByLabelText("Paths checked (1)")).toBeInTheDocument();

    const physical = screen.getByLabelText("Same file conflicts (1) content");
    const advisory = screen.getByLabelText(
      "Cross-checkout overlaps (1) content",
    );
    expect(physical).toHaveTextContent("Board work");
    expect(physical).not.toHaveTextContent("Peer work");
    expect(advisory).toHaveTextContent("Peer work");
    expect(advisory).not.toHaveTextContent("Board work");

    expect(
      screen.getByText(/touch the same physical file/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/same repository-relative path in a separate checkout/i),
    ).toBeInTheDocument();
  });

  it("describes an advisory overlap without claiming a conflict or a lock", () => {
    render(
      <WorkToolView
        content={inspection({
          rows: [peerRow],
          total: 1,
          returned: 1,
          integration_overlaps: ["trajectory:peer"],
        })}
        pending={false}
        failed={false}
      />,
    );
    expect(
      screen.queryByLabelText("Same file conflicts (1)"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(
        "These work items touch the same repository-relative path in a separate checkout. That is an integration warning, not a file conflict and not a lock.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/can overwrite their changes/i),
    ).not.toBeInTheDocument();
  });

  it("bounds a large overlap set behind the shared pager", () => {
    const many = Array.from({ length: 120 }, (_, index) => ({
      ...workFixture.rows[0],
      key: { runtime: "trajectory", id: `peer-${index}` },
      title: `Overlapping work item number ${index}`,
    }));
    render(
      <WorkToolView
        content={inspection({ rows: many, total: 120, returned: 120 })}
        pending={false}
        failed={false}
      />,
    );
    const body = screen.getByLabelText("Same file conflicts (120) content");
    expect(body.textContent?.length).toBeLessThanOrEqual(4096);
    expect(
      screen.getByRole("navigation", {
        name: "Same file conflicts (120) pages",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next section" })).toBeEnabled();
  });

  it("labels a withheld row and never invents a withheld count", () => {
    render(
      <WorkToolView
        content={inspection({
          rows: [{ ...peerRow, title: undefined, disclosure: "withheld" }],
          total: 1,
          returned: 1,
        })}
        pending={false}
        failed={false}
      />,
    );
    expect(
      screen.getAllByLabelText("Withheld by privacy policy").length,
    ).toBeGreaterThan(0);
    expect(
      screen.queryByLabelText("Withheld work count"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Peer work")).not.toBeInTheDocument();
  });

  it("reports a withheld count when the payload supplies one", () => {
    render(
      <WorkToolView
        content={JSON.stringify({ ...workFixture, withheld_rows: 1 })}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByLabelText("Withheld work count")).toHaveTextContent(
      "1 row was reduced by your privacy policy.",
    );
  });
});
