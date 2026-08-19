import { describe, expect, test, vi } from "vitest";
import { render, screen, fireEvent } from "../../utils/test-utils";
import { ErrorMessageCard } from "./ErrorMessage";
import type { ErrorMessage } from "../../services/refact/types";

describe("ErrorMessageCard", () => {
  test("renders unstructured errors as plain text instead of markdown subblocks", () => {
    const error: ErrorMessage = {
      role: "error",
      content:
        'Request failed\n- provider returned 500\n```json\n{"error":true}\n```',
    };

    const { container } = render(<ErrorMessageCard errors={[error]} />);

    expect(container).toHaveTextContent("Request failed");
    expect(container).toHaveTextContent("- provider returned 500");
    expect(container).toHaveTextContent('{"error":true}');
    expect(container.querySelector("pre")).not.toBeInTheDocument();
    expect(container.querySelector("ul")).not.toBeInTheDocument();
  });

  test("keeps a single structured error flat inside one card", () => {
    const error: ErrorMessage = {
      role: "error",
      content: "provider overloaded",
      error_info: {
        category: "ProviderTransient",
        title: "Provider is busy",
        explanation: "The provider is temporarily overloaded.",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: "HTTP 503\n- overloaded",
      },
    };

    const { container } = render(<ErrorMessageCard errors={[error]} />);

    expect(screen.getByText("Provider is busy")).toBeInTheDocument();
    expect(screen.getByText("Temporary provider issue")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    expect(container).toHaveTextContent("HTTP 503");
    expect(container).toHaveTextContent("- overloaded");
    expect(container.querySelector("pre")).not.toBeInTheDocument();
    expect(container.querySelector("ul")).not.toBeInTheDocument();
  });

  test("clicking Retry on a StreamCorrupted error calls onRetryGeneration once", () => {
    const onRetryGeneration = vi.fn();
    const error: ErrorMessage = {
      role: "error",
      content: "stream corrupted",
      error_info: {
        category: "StreamCorrupted",
        title: "Response stream broke",
        explanation: "The response stream ended unexpectedly.",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: "unexpected end of stream",
      },
    };

    render(
      <ErrorMessageCard
        errors={[error]}
        onRetryGeneration={onRetryGeneration}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Retry" }));

    expect(onRetryGeneration).toHaveBeenCalledTimes(1);
  });

  test("does not render a Retry action without a callback", () => {
    const error: ErrorMessage = {
      role: "error",
      content: "stream corrupted",
      error_info: {
        category: "StreamCorrupted",
        title: "Response stream broke",
        explanation: "The response stream ended unexpectedly.",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: "unexpected end of stream",
      },
    };

    render(<ErrorMessageCard errors={[error]} />);

    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
  });

  test("does not render Retry when the error is not retryable", () => {
    const onRetryGeneration = vi.fn();
    const error: ErrorMessage = {
      role: "error",
      content: "stream corrupted",
      error_info: {
        category: "StreamCorrupted",
        title: "Response stream broke",
        explanation: "The response stream ended unexpectedly.",
        suggested_action: "retry",
        is_retryable: false,
      },
    };

    render(
      <ErrorMessageCard
        errors={[error]}
        onRetryGeneration={onRetryGeneration}
      />,
    );

    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    expect(onRetryGeneration).not.toHaveBeenCalled();
  });

  test("does not render a clickable Retry while auto-retry is in progress", () => {
    const onRetryGeneration = vi.fn();
    const error: ErrorMessage = {
      role: "error",
      content: "stream corrupted",
      error_info: {
        category: "StreamCorrupted",
        title: "Response stream broke",
        explanation: "The response stream ended unexpectedly.",
        suggested_action: "retry",
        is_retryable: true,
        raw_error: "unexpected end of stream",
      },
      retry_status: {
        attempt: 1,
        max_attempts: 3,
        delay_secs: 2,
        in_progress: true,
      },
    };

    render(
      <ErrorMessageCard
        errors={[error]}
        onRetryGeneration={onRetryGeneration}
      />,
    );

    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    expect(onRetryGeneration).not.toHaveBeenCalled();
  });

  test("suppresses Retry for every grouped error while auto-retry is active", () => {
    const onRetryGeneration = vi.fn();
    const retryingError: ErrorMessage = {
      role: "error",
      content: "provider busy",
      error_info: {
        category: "ProviderTransient",
        title: "Provider is busy",
        explanation: "The provider is temporarily overloaded.",
        suggested_action: "retry",
        is_retryable: true,
      },
      retry_status: {
        attempt: 2,
        max_attempts: 5,
        delay_secs: 15,
        in_progress: true,
      },
    };
    const corruptedError: ErrorMessage = {
      role: "error",
      content: "stream corrupted",
      error_info: {
        category: "StreamCorrupted",
        title: "Response stream broke",
        explanation: "The response stream ended unexpectedly.",
        suggested_action: "retry",
        is_retryable: true,
      },
    };

    render(
      <ErrorMessageCard
        errors={[retryingError, corruptedError]}
        onRetryGeneration={onRetryGeneration}
      />,
    );

    expect(
      screen.queryByRole("button", { name: "Retry" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Retrying 15s · 2/5")).toBeInTheDocument();
    expect(onRetryGeneration).not.toHaveBeenCalled();
  });
});
