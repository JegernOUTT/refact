import { describe, expect, test, vi } from "vitest";
import { render, screen, fireEvent } from "../../utils/test-utils";
import { ErrorMessageCard } from "./ErrorMessage";
import type { ErrorMessage } from "../../services/refact/types";

const classified: ErrorMessage = {
  role: "error",
  content: "HTTP 503 actual provider error",
  error_info: {
    category: "ProviderTransient",
    title: "Provider busy",
    explanation: "Please try again",
    suggested_action: "retry",
    is_retryable: true,
    raw_error: "HTTP 503 actual provider error",
  },
};

describe("ErrorMessageCard", () => {
  test("keeps raw error equal to content visible exactly once", () => {
    render(<ErrorMessageCard errors={[classified, classified]} />);
    expect(screen.getAllByText(classified.content)).toHaveLength(1);
    expect(screen.queryByText("Please try again")).not.toBeInTheDocument();
  });
  test("keeps every distinct full error visible without markdown interpretation", () => {
    const raw = `provider\n${"long".repeat(120)}\nlast line`;
    const { container } = render(
      <ErrorMessageCard
        errors={[
          classified,
          { role: "error", content: raw },
          { role: "error", content: "```json\n- second error\n```" },
        ]}
      />,
    );
    expect(container.textContent).toContain(raw);
    expect(screen.getByRole("alert")).toHaveTextContent("second error");
    expect(container.querySelector("pre, ul, details")).toBeNull();
  });
  test("offers one manual retry only when a callback is supplied", () => {
    const retry = vi.fn();
    render(
      <ErrorMessageCard
        errors={[classified, { role: "error", content: "other failure" }]}
        onRetryGeneration={retry}
      />,
    );
    expect(screen.getAllByRole("button", { name: "Retry" })).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(retry).toHaveBeenCalledOnce();
  });
  test("legacy errors remain readable without invented counts", () => {
    render(
      <ErrorMessageCard
        errors={[{ role: "error", content: "legacy plain failure" }]}
      />,
    );
    expect(screen.getByText("legacy plain failure")).toBeVisible();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    expect(screen.queryByText(/used|sent|remaining/)).not.toBeInTheDocument();
  });
});
