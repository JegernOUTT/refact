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
