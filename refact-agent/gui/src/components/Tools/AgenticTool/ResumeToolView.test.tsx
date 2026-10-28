import { describe, expect, test } from "vitest";
import { render, screen } from "../../../utils/test-utils";
import { ResumeToolView } from "./ResumeToolView";
import { resumeReceipt } from "./resumeFixtures";

describe("ResumeToolView", () => {
  test("shows the typed cap and eligibility without executing a resume", () => {
    render(<ResumeToolView content={JSON.stringify(resumeReceipt())} />);
    expect(screen.getByLabelText("Execution cap")).toHaveTextContent("20 / 20");
    expect(
      screen.getByText("Eligibility").nextElementSibling,
    ).toHaveTextContent("Can raise the step cap");
    expect(screen.queryByRole("button")).toBeNull();
  });

  test("shows a queued lifecycle receipt without a cap", () => {
    render(
      <ResumeToolView
        lifecycle
        content={JSON.stringify(
          resumeReceipt({
            action: "recover",
            status: "queued",
            cap_stop: null,
            can_raise_and_resume: false,
          }),
        )}
      />,
    );
    expect(screen.getByText("State").nextElementSibling).toHaveTextContent(
      "Queued",
    );
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
      />,
    );
    expect(
      screen.getByText(/Saved runtime configuration unavailable/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Original execution scope unavailable/),
    ).toBeInTheDocument();
  });

  test.each([
    "legacy",
    "{}",
    JSON.stringify(resumeReceipt({ change_seq: -1 })),
    JSON.stringify(resumeReceipt({ cap_stop: { key: "provider_quota" } })),
    JSON.stringify(resumeReceipt({ can_resume: "true" })),
    JSON.stringify(resumeReceipt({ action: ["inspect"] })),
    JSON.stringify(resumeReceipt({ run_generation: 4 })),
  ])("renders nothing for malformed or legacy receipts %s", (content) => {
    render(<ResumeToolView content={content} />);
    expect(screen.queryByLabelText("Child resume")).toBeNull();
  });
});
