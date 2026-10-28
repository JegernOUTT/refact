import { describe, expect, it } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { ApprovalToolView } from "./ApprovalToolView";

describe("Child approval view", () => {
  it("renders a decision receipt without raw keys", () => {
    render(
      <ApprovalToolView
        content={JSON.stringify({
          request_id: "request",
          decision: "approve",
          already_decided: true,
        })}
      />,
    );
    expect(screen.getByLabelText("Child command approval")).toHaveTextContent(
      "request",
    );
    expect(screen.queryByText(/already_decided/)).not.toBeInTheDocument();
  });

  it("lists every request snapshot with its decision", () => {
    render(
      <ApprovalToolView
        content={JSON.stringify({
          requests: [
            { request_id: "first", decision: "pending" },
            { request_id: "second", decision: "approved", consumed: true },
          ],
        })}
      />,
    );
    expect(screen.getByText("first")).toBeInTheDocument();
    expect(screen.getByText("second")).toBeInTheDocument();
    expect(screen.getByText("consumed")).toBeInTheDocument();
  });

  it("keeps empty and unreadable receipts distinct", () => {
    const view = render(<ApprovalToolView content='{"requests":[]}' />);
    expect(screen.getByText(/No approval requests/)).toBeInTheDocument();
    for (const content of [
      "legacy text",
      '{"request_id":"r","decision":"completed"}',
      '{"requests":[{}]}',
    ]) {
      view.rerender(<ApprovalToolView content={content} />);
      expect(screen.queryByLabelText("Child command approval")).toBeNull();
    }
  });
});
