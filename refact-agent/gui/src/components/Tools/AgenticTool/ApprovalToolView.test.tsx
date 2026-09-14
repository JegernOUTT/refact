import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";
import { render, screen } from "../../../utils/test-utils";
import { ApprovalToolView } from "./ApprovalToolView";

describe("Child approval card", () => {
  it("renders an idempotent receipt without claiming command completion", () => {
    render(
      <ApprovalToolView
        content={JSON.stringify({
          request_id: "request",
          decision: "approve",
          already_decided: true,
        })}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByLabelText("Child command approval")).toHaveTextContent(
      "Repeated decision acknowledged",
    );
    expect(screen.getByText(/not an execution result/)).toBeInTheDocument();
    expect(screen.queryByText(/already_decided/)).not.toBeInTheDocument();
  });

  it("paginates request snapshots instead of mounting an unbounded list", () => {
    render(
      <ApprovalToolView
        content={JSON.stringify({
          requests: [
            { request_id: "first", decision: "pending" },
            { request_id: "second", decision: "approved", consumed: true },
          ],
        })}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText("first")).toBeInTheDocument();
    expect(screen.queryByText("second")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Next request" }));
    expect(screen.getByText("second")).toBeInTheDocument();
    expect(
      screen.getByText(/does not prove command success/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next request" })).toBeDisabled();
  });

  it("keeps pending, empty, legacy and failed outcomes distinct", () => {
    const view = render(
      <ApprovalToolView content={null} pending failed={false} />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Waiting");
    view.rerender(
      <ApprovalToolView
        content='{"requests":[]}'
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText(/No approval requests/)).toBeInTheDocument();
    for (const content of [
      "legacy text",
      '{"request_id":"r","decision":"completed"}',
      '{"requests":[{}]}',
    ]) {
      view.rerender(
        <ApprovalToolView content={content} pending={false} failed={false} />,
      );
      expect(screen.getByRole("alert")).toHaveTextContent(
        "does not confirm permission",
      );
    }
    view.rerender(
      <ApprovalToolView
        content='{"request_id":"r","decision":"approved"}'
        pending={false}
        failed
      />,
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});
