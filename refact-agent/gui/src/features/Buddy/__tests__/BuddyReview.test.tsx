import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "../../../utils/test-utils";
import { CommitNumberField } from "../BuddySettingsFields";
import { BuddyRow, BuddyTimestamp } from "../ui";
import { fenceMachinePayloads } from "../BuddyArtifactBodyText";

describe("Buddy review regressions", () => {
  it.each(["unmount", "blur", "escape", "invalid"])(
    "number draft %s",
    (mode) => {
      const commit = vi.fn((raw: string) => raw === "7");
      const result = render(
        <CommitNumberField
          value={18}
          onCommit={commit}
          ariaLabel="Hour"
          min={0}
          max={23}
        />,
      );
      const input = screen.getByRole("spinbutton");
      fireEvent.change(input, {
        target: { value: mode === "invalid" ? "99" : "7" },
      });
      if (mode === "blur") fireEvent.blur(input);
      if (mode === "escape") fireEvent.keyDown(input, { key: "Escape" });
      result.unmount();
      if (mode === "escape" || mode === "invalid")
        expect(commit).not.toHaveBeenCalled();
      else expect(commit.mock.calls).toEqual([["7"]]);
    },
  );

  it.each(["button", "link"])(
    "timestamps inside row %s have no tab stop",
    (kind) => {
      const { container } = render(
        <BuddyRow
          title="Report"
          onClick={kind === "button" ? () => undefined : undefined}
          href={kind === "link" ? "/report" : undefined}
          meta={[<BuddyTimestamp key="time" value="2026-01-01T12:00:00Z" />]}
        />,
      );
      const row = container.querySelector("button, a");
      expect(row?.querySelector("[tabindex]")).toBeNull();
      expect(row?.querySelector("time")).toHaveAttribute(
        "datetime",
        "2026-01-01T12:00:00.000Z",
      );
      expect(row?.querySelector("time")).toHaveAccessibleName(/2026/);
    },
  );

  it("standalone timestamps keep their tab stop", () => {
    const { container } = render(
      <BuddyTimestamp value="2026-01-01T12:00:00Z" />,
    );
    expect(container.querySelector("time")).toHaveAttribute("tabindex", "0");
  });

  it("keeps prose with configuration pairs out of code fences", () => {
    const prose = "Use timeout=30 and retries=3 to avoid transient failures.";
    expect(fenceMachinePayloads(prose)).toBe(prose);
  });

  it("fences record-shaped memory payloads", () => {
    const record =
      "memory_proposal id=79a92f11-1111-1111-1111-111111111111 count=5 status=pending";
    expect(fenceMachinePayloads(record)).toBe("```text\n" + record + "\n```");
  });
});
