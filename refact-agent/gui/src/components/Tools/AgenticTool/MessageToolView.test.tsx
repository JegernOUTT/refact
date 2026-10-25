import { describe, expect, test } from "vitest";
import { fireEvent, render, screen } from "../../../utils/test-utils";
import { MessageToolView } from "./MessageToolView";
import { buildCompactLaunchMessagePresentation } from "./compactLaunchMessage";
import { boardReceipt, receiptState } from "./messageReceipt";

function view(
  name: string,
  args: Record<string, unknown>,
  content: string | null,
  pending = false,
  failed = false,
) {
  return render(
    <MessageToolView
      toolName={name}
      argumentsRaw={JSON.stringify(args)}
      content={content}
      pending={pending}
      failed={failed}
    />,
  );
}

describe("Interaction receipts", () => {
  test.each([
    ["queued", "Queued"],
    ["delivered", "Delivered"],
    ["consumed", "Consumed"],
    ["duplicate", "Duplicate"],
    ["expired", "Expired"],
    ["superseded", "Superseded"],
    ["failed", "Delivery failed"],
  ])("compact and detailed mail share the %s receipt", (outcome, compact) => {
    const args = { to: "child", text: "Review parser" };
    const extra = {
      delivery_receipt: {
        kind: "agent_message",
        delivery_id: "d-1",
        outcome,
        ttl_secs: 30,
      },
    };
    const receipt = receiptState(
      "agent_message",
      args,
      null,
      boardReceipt("agent_message", extra),
    );
    render(
      <MessageToolView
        toolName="agent_message"
        argumentsRaw={JSON.stringify(args)}
        content={null}
        extra={extra}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText(receipt.label)).toBeInTheDocument();
    expect(
      buildCompactLaunchMessagePresentation({
        toolName: "agent_message",
        arguments: args,
        extra,
        transportState: "succeeded",
      })?.stateLabel,
    ).toBe(compact);
  });

  test.each([
    [
      "agent_message",
      { to: "child", text: "hello" },
      "Message queued for child.",
      "Message queued",
    ],
    [
      "agent_message",
      { to: "parent", text: "hello" },
      "Note sent to parent.",
      "Note accepted",
    ],
    [
      "agent_message",
      { to: "parent", text: "hello", expects_reply: true },
      "Question q-123 sent to parent.",
      "Question recorded",
    ],
    [
      "agent_message",
      { to: "child", text: "hello", reply_to: "q-123" },
      "Answer recorded for finished agent child.",
      "Answer recorded",
    ],
    [
      "progress_report",
      { status_line: "done" },
      "Progress updated: done",
      "Progress recorded",
    ],
    [
      "t_agent_message",
      { to: "child", text: "hello" },
      "Message queued for child.",
      "Message queued",
    ],
  ])(
    "recognizes %s receipt without claiming completion",
    (name, args, content, label) => {
      view(name, args, content);
      expect(screen.getByText(label)).toBeInTheDocument();
      expect(screen.queryByText("Verified")).toBeNull();
      expect(screen.queryByTestId("agentic-tool-raw-args")).toBeNull();
    },
  );

  test.each([
    [
      "agent_message",
      { to: "child", text: "hello" },
      "Message queued for other.",
    ],
    [
      "agent_message",
      { to: "parent", text: "hello" },
      "Question q-123 sent to parent.",
    ],
    ["progress_report", { status_line: "old" }, "Progress updated: changed"],
    ["agent_message", {}, '{"status":"completed"}'],
    ["task_broadcast", { message: "hello" }, "Broadcast sent to 5 agents"],
    ["agent_steer", { message: "hello", card_id: "card" }, "Steered card"],
  ])(
    "does not infer delivery from unsupported %s response",
    (name, args, content) => {
      view(name, args, content);
      expect(screen.getByText("Outcome unconfirmed")).toBeInTheDocument();
      expect(
        screen.getByLabelText("Unconfirmed response content"),
      ).toHaveTextContent(content);
    },
  );

  test.each([
    ["queued", 30, "Message queued (expires after 30s unless landed)"],
    ["queued", null, "Message queued"],
    ["expired", 5, "the deadline passed before any boundary opened"],
    ["superseded", null, "replaced by a newer message with the same key"],
    ["duplicate", null, "duplicate of an already-sent message"],
    ["consumed", null, "Message consumed by the recipient"],
    ["failed", null, "delivery failed before consumption"],
  ] as const)(
    "structured agent_message receipt %s/%s is labelled truthfully",
    (outcome, ttl, label) => {
      render(
        <MessageToolView
          toolName="agent_message"
          argumentsRaw={JSON.stringify({ to: "child", text: "hello" })}
          content="Message queued for child."
          extra={{
            delivery_receipt: {
              kind: "agent_message",
              delivery_id: "d-1",
              outcome,
              ttl_secs: ttl,
            },
          }}
          pending={false}
          failed={false}
        />,
      );
      expect(
        screen.getByText(
          new RegExp(label.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&"), "u"),
        ),
      ).toBeInTheDocument();
      expect(
        screen.queryByLabelText("Unconfirmed response content"),
      ).toBeNull();
    },
  );

  test("malformed agent_message receipts fall back to the conservative text rule", () => {
    render(
      <MessageToolView
        toolName="agent_message"
        argumentsRaw={JSON.stringify({ to: "child", text: "hello" })}
        content="Message queued for child."
        extra={{
          delivery_receipt: {
            kind: "agent_message",
            delivery_id: "",
            outcome: "landed",
          },
        }}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText("Message queued")).toBeInTheDocument();
  });

  test("renders structured steer receipts without treating acceptance as consumption", () => {
    render(
      <MessageToolView
        toolName="agent_steer"
        argumentsRaw={JSON.stringify({ card_id: "card-1", message: "steer" })}
        content="✅ Steered card-1"
        extra={{
          delivery_receipt: {
            kind: "agent_steer",
            card_id: "card-1",
            outcome: "queued",
            reached: true,
            prior_state: "generating",
          },
        }}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText("Queued for the agent")).toBeInTheDocument();
    expect(screen.getByText("generating")).toBeInTheDocument();
    expect(screen.queryByLabelText("Unconfirmed response content")).toBeNull();
    expect(screen.getByText(/not proof of consumption/u)).toBeInTheDocument();
  });

  test.each([
    {
      kind: "agent_steer",
      card_id: "card-1",
      outcome: "queued",
      reached: false,
    },
    {
      kind: "agent_steer",
      card_id: "card-1",
      outcome: "teleported",
      reached: true,
    },
    {
      kind: "task_broadcast",
      card_id: "card-1",
      outcome: "queued",
      reached: true,
    },
    "queued",
  ])("ignores inconsistent or malformed steer receipts %#", (receipt) => {
    render(
      <MessageToolView
        toolName="agent_steer"
        argumentsRaw={JSON.stringify({ card_id: "card-1", message: "steer" })}
        content="Steered card-1"
        extra={{ delivery_receipt: receipt }}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText("Outcome unconfirmed")).toBeInTheDocument();
  });

  test("pages broadcast receipts per card and counts only reached agents", () => {
    const cards = Array.from({ length: 7 }, (_, index) => ({
      card_id: `card-${index}`,
      title: `Card ${index}`,
      outcome: index % 2 === 0 ? "delivered" : "no_live_session",
      reached: index % 2 === 0,
      reason: index % 2 === 0 ? null : "agent session closed",
    }));
    render(
      <MessageToolView
        toolName="task_broadcast"
        argumentsRaw={JSON.stringify({ message: "everyone pause" })}
        content="Broadcast sent"
        extra={{
          delivery_receipts: {
            kind: "task_broadcast",
            notified: 4,
            total: 7,
            cards,
          },
        }}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText("4 of 7 board agents reached")).toBeInTheDocument();
    const receipts = screen.getByLabelText("Board delivery receipts");
    expect(receipts).toHaveTextContent("Card 0: Delivered to the agent");
    expect(receipts).toHaveTextContent(
      "Card 1: Not delivered: no live agent session (agent session closed)",
    );
    expect(receipts).not.toHaveTextContent("Card 5");
    fireEvent.click(screen.getByRole("button", { name: "Next cards" }));
    expect(receipts).toHaveTextContent("Card 5");
    expect(screen.getByRole("button", { name: "Next cards" })).toBeDisabled();
  });

  test("rejects broadcast receipts whose counts disagree with the cards", () => {
    render(
      <MessageToolView
        toolName="task_broadcast"
        argumentsRaw={JSON.stringify({ message: "hello" })}
        content="Broadcast sent"
        extra={{
          delivery_receipts: {
            kind: "task_broadcast",
            notified: 2,
            total: 1,
            cards: [{ card_id: "card-1", outcome: "delivered", reached: true }],
          },
        }}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByText("Outcome unconfirmed")).toBeInTheDocument();
  });

  test("pages message and legacy response separately and leaves markup inert", () => {
    view(
      "agent_message",
      {
        to: "child",
        text: "A".repeat(4096) + "Second message<script>bad()</script>",
      },
      "B".repeat(4096) + "Second response",
    );
    const body = screen.getByLabelText("Requested message content");
    expect(body.textContent).toHaveLength(4096);
    fireEvent.click(screen.getAllByRole("button", { name: "Next section" })[0]);
    expect(body).toHaveTextContent("Second message<script>bad()</script>");
    expect(body.querySelector("script")).toBeNull();
    expect(
      screen.getByLabelText("Unconfirmed response content").textContent,
    ).toHaveLength(4096);
    fireEvent.click(screen.getAllByRole("button", { name: "Next section" })[1]);
    expect(
      screen.getByLabelText("Unconfirmed response content"),
    ).toHaveTextContent("Second response");
  });

  test("failure overrides success-looking text", () => {
    view(
      "agent_message",
      { to: "child", text: "hello" },
      "Message queued for child.",
      false,
      true,
    );
    expect(screen.getByText("Request failed")).toBeInTheDocument();
    expect(screen.queryByText("Message queued")).toBeNull();
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  test("pending receipt does not expose an incomplete reply as a confirmed question", () => {
    view(
      "agent_message",
      { to: "parent", text: "hello", expects_reply: true },
      "Question q-123 sent to parent.",
      true,
    );
    expect(screen.getByText("Awaiting receipt")).toBeInTheDocument();
    expect(screen.queryByText("q-123")).toBeNull();
    expect(screen.queryByLabelText("Unconfirmed response content")).toBeNull();
  });
});
