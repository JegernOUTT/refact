import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "../../../utils/test-utils";
import { EventRow } from "./EventRow";
import { eventSummary } from "./eventSummary";
import { eventDetailFields } from "./eventDetails";
import {
  eventContentAttachments,
  eventContentText,
  isVisibleCompressionFailureEvent,
  type EventMessage,
} from "../../../services/refact/types";

const IMAGE = "c3ludGhldGlj";

function event(content: EventMessage["content"]): EventMessage {
  return {
    role: "event",
    message_id: "event-1",
    subkind: "system_notice",
    source: "tool_job.completed",
    content,
    payload: { actor: "tool_completion" },
  };
}

const textAndImage = event([
  { m_type: "text", m_content: "Synthetic\n capture" },
  { m_type: "image/png", m_content: IMAGE },
]);
const imageOnly = event([{ m_type: "image/png", m_content: IMAGE }]);

describe("event content projection", () => {
  it("projects text parts and keeps image attachments separate", () => {
    expect(eventContentText(textAndImage.content)).toBe("Synthetic\n capture");
    expect(eventContentText(imageOnly.content)).toBe("");
    expect(eventContentText("plain")).toBe("plain");
    expect(eventContentAttachments(textAndImage.content)).toEqual([
      { m_type: "image/png", m_content: IMAGE },
    ]);
    expect(eventContentAttachments("plain")).toEqual([]);
  });

  it("never leaks image data into summaries or details", () => {
    expect(eventSummary(textAndImage)).toBe("Synthetic capture");
    expect(eventSummary(imageOnly)).toBe("");
    const details = JSON.stringify(eventDetailFields(textAndImage));
    expect(details).not.toContain(IMAGE);
  });

  it("checks compression failures on projected text", () => {
    const failure = {
      ...event([
        { m_type: "text", m_content: "Context compression failed: x" },
      ]),
      source: "chat.summarizer",
      extra: { event: { subkind: "system_notice", source: "chat.summarizer" } },
    };
    expect(isVisibleCompressionFailureEvent(failure)).toBe(true);
  });
});

describe("EventRow multimodal content", () => {
  it("renders text and image content without crashing", () => {
    render(<EventRow event={textAndImage} />);
    expect(screen.getByTestId("transcript-target").textContent).toBe(
      "Synthetic capture",
    );
    expect(screen.getByTestId("transcript-meta").textContent).toContain(
      "1 image",
    );
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    const images = screen.getByTestId("event-images").querySelectorAll("img");
    expect(images).toHaveLength(1);
    expect(images[0].getAttribute("src")).toBe(
      `data:image/png;base64,${IMAGE}`,
    );
    expect(screen.getByTestId("transcript-target").textContent).not.toContain(
      IMAGE,
    );
  });

  it("renders image-only content", () => {
    render(<EventRow event={imageOnly} />);
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(
      screen.getByTestId("event-images").querySelectorAll("img"),
    ).toHaveLength(1);
  });

  it("keeps ordinary text events unchanged", () => {
    render(<EventRow event={event("Plain notice")} />);
    expect(screen.getByTestId("transcript-target").textContent).toBe(
      "Plain notice",
    );
    expect(screen.queryByTestId("event-images")).toBeNull();
  });
});
