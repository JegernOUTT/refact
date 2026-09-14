import { describe, expect, test, vi } from "vitest";
import { render, screen, fireEvent } from "../../../utils/test-utils";
import { ResumeToolView } from "./ResumeToolView";
import { resumeReceipt } from "./resumeFixtures";

describe("ResumeToolView", () => {
  test("shows typed cap and revision without executing a resume", () => {
    const open = vi.fn();
    render(
      <ResumeToolView
        content={JSON.stringify(resumeReceipt())}
        pending={false}
        failed={false}
        onOpenChildTrajectory={open}
      />,
    );
    expect(screen.getByLabelText("Execution cap")).toHaveTextContent("20 / 20");
    expect(screen.getByText(/Inspection only/)).toBeInTheDocument();
    expect(
      screen.getByText("Record revision").nextElementSibling,
    ).toHaveTextContent("14");
    expect(
      screen.getByText(/Unknown tool outcomes are never replayed/),
    ).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Open child trajectory" }),
    );
    expect(open).toHaveBeenCalledExactlyOnceWith("child-trajectory");
  });
  test("distinguishes a queued resume receipt from execution success", () => {
    render(
      <ResumeToolView
        content={JSON.stringify(
          resumeReceipt({
            action: "resume",
            status: "queued",
            cap_stop: null,
            can_raise_and_resume: false,
          }),
        )}
        pending={false}
        failed={false}
        onOpenChildTrajectory={vi.fn()}
      />,
    );
    expect(
      screen.getByText(/Queued is not running or completed/),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Execution cap")).toBeNull();
  });
  test("reports unavailable saved scope and configuration explicitly", () => {
    render(
      <ResumeToolView
        content={JSON.stringify(
          resumeReceipt({
            original_scope_available: false,
            resume_configuration_available: false,
            child_chat_id: null,
            max_steps: null,
            settings_subagent_max_steps_ceiling: null,
            can_raise_and_resume: false,
          }),
        )}
        pending={false}
        failed={false}
        onOpenChildTrajectory={vi.fn()}
      />,
    );
    expect(
      screen.getByText(/Saved runtime configuration unavailable/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Original execution scope unavailable/),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Settings step ceiling").nextElementSibling,
    ).toHaveTextContent("Unavailable");
    expect(screen.queryByRole("button")).toBeNull();
  });
  test.each([
    "legacy",
    "{}",
    JSON.stringify(resumeReceipt({ change_seq: -1 })),
    JSON.stringify(resumeReceipt({ cap_stop: { key: "provider_quota" } })),
    JSON.stringify(resumeReceipt({ can_resume: "true" })),
    JSON.stringify(resumeReceipt({ action: ["inspect"] })),
    JSON.stringify(resumeReceipt({ run_generation: 4 })),
  ])("fails closed for malformed or legacy receipts %s", (content) => {
    render(
      <ResumeToolView
        content={content}
        pending={false}
        failed={false}
        onOpenChildTrajectory={vi.fn()}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("does not confirm");
    expect(screen.queryByRole("button")).toBeNull();
  });
  test("keeps pending and failed results distinct", () => {
    const { rerender } = render(
      <ResumeToolView
        content={null}
        pending
        failed={false}
        onOpenChildTrajectory={vi.fn()}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Waiting");
    rerender(
      <ResumeToolView
        content={JSON.stringify(resumeReceipt())}
        pending={false}
        failed
        onOpenChildTrajectory={vi.fn()}
      />,
    );
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});
