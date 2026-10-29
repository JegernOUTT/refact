import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  BUDDY_OPERATOR_STATUS,
  BUDDY_STATUS_MAPS,
  buddyStatus,
  toBuddyHealth,
  toBuddySeverity,
  toBuddyWorkState,
} from "../buddyStatus";
import { BuddyStatusBadge } from "../BuddyStatusBadge";
import { buddyLoadError, buddyName, buddyPossessive } from "../buddyGlossary";

const ALLOWED_TONES = ["default", "success", "warning", "danger"];

describe("buddy status maps", () => {
  it("gives every key in every dimension a label and an allowed tone", () => {
    for (const map of Object.values(BUDDY_STATUS_MAPS)) {
      for (const entry of Object.values(map)) {
        expect(entry.label.trim()).not.toBe("");
        expect(ALLOWED_TONES).toContain(entry.tone);
      }
    }
    for (const entry of Object.values(BUDDY_OPERATOR_STATUS)) {
      expect(entry.label.trim()).not.toBe("");
      expect(ALLOWED_TONES).toContain(entry.tone);
    }
  });

  it("covers the required keys", () => {
    expect(Object.keys(BUDDY_STATUS_MAPS.work).sort()).toEqual(
      [
        "blocked",
        "cancelled",
        "delivered",
        "needs_you",
        "snoozed",
        "watching",
        "working",
      ].sort(),
    );
    expect(Object.keys(BUDDY_STATUS_MAPS.severity)).toEqual([
      "critical",
      "high",
      "medium",
      "low",
    ]);
    expect(Object.keys(BUDDY_STATUS_MAPS.read)).toEqual(["unread", "read"]);
    expect(Object.keys(BUDDY_STATUS_MAPS.enabled)).toEqual([
      "enabled",
      "disabled",
    ]);
  });

  it("never uses accent for a status", () => {
    const all = [
      ...Object.values(BUDDY_STATUS_MAPS).flatMap((m) => Object.values(m)),
      ...Object.values(BUDDY_OPERATOR_STATUS),
    ];
    expect(all.some((entry) => (entry.tone as string) === "accent")).toBe(
      false,
    );
  });

  it("maps backend keys", () => {
    expect(toBuddyWorkState("needs_attention")).toBe("needs_you");
    expect(buddyStatus("work", "needs_you").label).toBe("Needs you");
    for (const key of [
      "needs_attention",
      "blocked",
      "working",
      "delivered",
      "watching",
      "cancelled",
    ]) {
      expect(toBuddyWorkState(key)).not.toBeNull();
    }
    expect(toBuddyWorkState("bogus")).toBeNull();
    expect(toBuddySeverity("normal")).toBe("medium");
    expect(toBuddySeverity("unknown")).toBeNull();
    expect(toBuddyHealth("unavailable")).toBe("failed");
    expect(toBuddyHealth(null)).toBeNull();
  });
});

describe("BuddyStatusBadge", () => {
  it("renders a work badge with label and tone", () => {
    render(<BuddyStatusBadge dimension="work" value="delivered" />);
    const badge = screen.getByText("Delivered");
    expect(badge).toHaveAttribute("data-tone", "success");
    expect(badge).toHaveAttribute("data-dimension", "work");
  });

  it("renders severity as a dot plus text", () => {
    render(
      <BuddyStatusBadge dimension="severity" value="critical" testId="sev" />,
    );
    const severity = screen.getByTestId("sev");
    expect(severity).toHaveTextContent("Critical");
    expect(severity).toHaveAttribute("data-tone", "danger");
    expect(severity.querySelector("span")).not.toBeNull();
  });

  it("accepts a label override", () => {
    render(<BuddyStatusBadge dimension="read" value="unread" label="New" />);
    expect(screen.getByText("New")).toBeInTheDocument();
  });
});

describe("buddy glossary", () => {
  it("builds load errors and names", () => {
    expect(buddyLoadError("reports")).toBe("Couldn't load reports.");
    expect(buddyName("  ")).toBe("Buddy");
    expect(buddyName("Pixel")).toBe("Pixel");
    expect(buddyPossessive("Pixel")).toBe("Pixel's");
    expect(buddyPossessive("Gus")).toBe("Gus'");
  });
});
