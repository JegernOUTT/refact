import { describe, expect, it } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { WorkToolView } from "./WorkToolView";
import { workFixture } from "../../../services/refact/workFixtures";

describe("Work tool view", () => {
  it("renders structured states rather than raw JSON", () => {
    render(<WorkToolView content={JSON.stringify(workFixture)} />);
    expect(screen.getByText("Board work")).toBeInTheDocument();
    expect(screen.getByText("Queued")).toBeInTheDocument();
    expect(screen.queryByText(/snapshot_id/)).not.toBeInTheDocument();
  });

  it("renders nothing for legacy or empty responses", () => {
    const view = render(<WorkToolView content={null} />);
    expect(screen.queryByLabelText("Work snapshot")).toBeNull();
    view.rerender(<WorkToolView content="legacy response" />);
    expect(screen.queryByLabelText("Work snapshot")).toBeNull();
  });

  it("does not claim emptiness for an inspected item with no children", () => {
    render(
      <WorkToolView
        content={JSON.stringify({
          ...workFixture,
          row: workFixture.rows[0],
          rows: [],
          total: 0,
          returned: 0,
        })}
      />,
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

describe("Work tool view file overlap", () => {
  it("renders no overlap section when nothing overlaps", () => {
    render(
      <WorkToolView
        content={inspection({ rows: [], total: 0, returned: 0 })}
      />,
    );
    expect(screen.queryByLabelText("File overlap")).not.toBeInTheDocument();
  });

  it("separates a physical file conflict from an advisory overlap", () => {
    render(
      <WorkToolView
        content={inspection({ integration_overlaps: ["trajectory:peer"] })}
      />,
    );
    expect(screen.getByLabelText("File overlap")).toHaveTextContent(
      "src/lib.rs",
    );
    const physical = screen.getByLabelText("Same file conflicts (1)");
    const advisory = screen.getByLabelText("Cross-checkout overlaps (1)");
    expect(physical).toHaveTextContent("Board work");
    expect(physical).not.toHaveTextContent("Peer work");
    expect(advisory).toHaveTextContent("Peer work");
    expect(advisory).not.toHaveTextContent("Board work");
  });

  it("labels a withheld row and never invents a withheld count", () => {
    render(
      <WorkToolView
        content={inspection({
          rows: [{ ...peerRow, title: undefined, disclosure: "withheld" }],
          total: 1,
          returned: 1,
        })}
      />,
    );
    expect(
      screen.getAllByText(/Withheld by privacy policy/).length,
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
      />,
    );
    expect(screen.getByLabelText("Withheld work count")).toHaveTextContent(
      "1 row was reduced by your privacy policy.",
    );
  });
});
