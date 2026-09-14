import { describe, expect, test } from "vitest";
import { render, screen, fireEvent } from "../../../utils/test-utils";
import { PlanToolView } from "./PlanToolView";

describe("PlanToolView", () => {
  test("paginates historical content and never promotes a receipt into verified success", () => {
    const { container } = render(
      <PlanToolView
        toolName="get_plan"
        pending={false}
        failed={false}
        content={JSON.stringify({
          plan: {
            content: "A".repeat(4096) + "<script>second section</script>",
            version: 1,
            mode: "agent",
            delta_count: 7,
          },
        })}
      />,
    );
    expect(screen.getByLabelText("Plan content").textContent).toHaveLength(
      4096,
    );
    expect(screen.queryByText(/second section/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Next section" }));
    expect(screen.getByLabelText("Plan content")).toHaveTextContent(
      "<script>second section</script>",
    );
    expect(container.querySelector("script")).toBeNull();
    expect(screen.getByRole("button", { name: "Next section" })).toBeDisabled();
    expect(screen.getByText(/does not establish approval/)).toBeInTheDocument();
  });

  test.each([
    ["set_plan", { version: 1, supersedes: null }, "base plan was accepted"],
    ["update_plan", { seq: 2, truncated: false }, "without truncation"],
    ["update_plan", { seq: 2, truncated: true }, "with truncation"],
    ["get_plan", { plan: null }, "No plan was installed"],
    ["t_set_plan", { version: 1, supersedes: null }, "base plan was accepted"],
  ])("renders %s receipt", (toolName, payload, text) => {
    render(
      <PlanToolView
        toolName={toolName}
        content={JSON.stringify(payload)}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByLabelText("Plan receipt")).toHaveTextContent(text);
  });

  test.each([
    "legacy response",
    "{}",
    '{"seq":-1,"truncated":false}',
    '{"seq":2,"truncated":"false"}',
  ])("fails closed for %s", (content) => {
    render(
      <PlanToolView
        toolName="update_plan"
        content={content}
        pending={false}
        failed={false}
      />,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("does not confirm");
  });

  test("keeps streaming and failed outcomes distinct", () => {
    const props = {
      toolName: "set_plan",
      content: '{"version":1,"supersedes":null}',
    };
    const view = render(<PlanToolView {...props} pending failed={false} />);
    expect(screen.getByRole("status")).toHaveTextContent(
      "No plan change confirmed",
    );
    view.rerender(<PlanToolView {...props} pending={false} failed />);
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });
});
