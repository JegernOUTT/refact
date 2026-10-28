import { describe, expect, test } from "vitest";
import { fireEvent, render, screen } from "../../../utils/test-utils";
import { MessageToolView } from "./MessageToolView";
import { boardReceipt } from "./messageReceipt";

function cards(count: number, unreached: number[] = []) {
  return Array.from({ length: count }, (_, index) => ({
    card_id: `T-${index}`,
    title: `Card ${index} <script>inert</script>`,
    outcome: unreached.includes(index) ? "no_live_session" : "delivered",
    reached: !unreached.includes(index),
    reason: unreached.includes(index) ? "agent session closed" : null,
  }));
}

describe("MessageToolView", () => {
  test("renders the message as inert markdown", () => {
    render(
      <MessageToolView
        toolName="agent_steer"
        argumentsRaw={JSON.stringify({
          card_id: "T-1",
          message: "**Pause** <script>bad()</script>",
        })}
        content="ok"
      />,
    );
    const body = screen.getByTestId("agentic-tool-message");
    expect(body).toHaveTextContent("Pause");
    expect(body.querySelector("strong")).not.toBeNull();
    expect(body.querySelector("script")).toBeNull();
  });

  test("shows steer delivery outcomes", () => {
    render(
      <MessageToolView
        toolName="agent_steer"
        argumentsRaw={JSON.stringify({ card_id: "T-1", message: "stop" })}
        content="ok"
        extra={{
          delivery_receipt: {
            kind: "agent_steer",
            card_id: "T-1",
            outcome: "no_live_session",
            reached: false,
          },
        }}
      />,
    );
    expect(
      screen.getByText("Not delivered: no live agent session"),
    ).toBeInTheDocument();
  });

  test("pages broadcast receipts per card and counts only reached agents", () => {
    const list = cards(7, [0, 3, 6]);
    render(
      <MessageToolView
        toolName="task_broadcast"
        argumentsRaw={JSON.stringify({ message: "Pause edits" })}
        content="Broadcast sent"
        extra={{
          delivery_receipts: {
            kind: "task_broadcast",
            notified: 4,
            total: 7,
            cards: list,
          },
        }}
      />,
    );
    expect(screen.getByText("4 of 7 board agents reached")).toBeInTheDocument();
    const receipts = screen.getByLabelText("Board delivery receipts");
    expect(receipts).toHaveTextContent("Card 0 <script>inert</script>");
    expect(receipts.querySelector("script")).toBeNull();
    expect(receipts).not.toHaveTextContent("Card 5");
    fireEvent.click(screen.getByRole("button", { name: "Next cards" }));
    expect(receipts).toHaveTextContent("Card 5");
    expect(screen.getByRole("button", { name: "Next cards" })).toBeDisabled();
  });

  test("rejects broadcast receipts whose counts disagree with the cards", () => {
    expect(
      boardReceipt("task_broadcast", {
        delivery_receipts: {
          kind: "task_broadcast",
          notified: 5,
          total: 2,
          cards: cards(2),
        },
      }),
    ).toBeNull();
  });

  test("falls back to the response text when no message body exists", () => {
    render(
      <MessageToolView
        toolName="planner_reply"
        argumentsRaw="{}"
        content="Reply stored"
      />,
    );
    expect(screen.getByText("Reply stored")).toBeInTheDocument();
  });
});
