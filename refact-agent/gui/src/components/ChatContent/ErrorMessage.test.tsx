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

  test("renders unclassified errors as an Error row", () => {
    render(
      <ErrorMessageCard
        errors={[{ role: "error", content: "429 usage limit reached" }]}
      />,
    );
    const card = screen.getByTestId("error-message-card");
    expect(card).toHaveAttribute("data-transcript-status", "failed");
    expect(card).toHaveAttribute("data-open", "true");
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Error");
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent("429");
    expect(screen.getByRole("alert")).toHaveTextContent(
      "429 usage limit reached",
    );
  });

  test("renders one failed row with the error callout open", () => {
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

    expect(screen.getByTestId("transcript-verb")).toHaveTextContent(
      "Request failed",
    );
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Provider is busy",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "503 · Temporary provider issue",
    );
    expect(screen.getAllByTestId("transcript-row")).toHaveLength(1);
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

  test("offers manual Retry for permanent model errors", () => {
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

    expect(screen.queryByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(onRetryGeneration).not.toHaveBeenCalled();
  });

  test("historical retry status does not suppress Retry", () => {
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

    expect(screen.queryByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(onRetryGeneration).not.toHaveBeenCalled();
  });

  test("grouped historical retry status is not live state", () => {
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

    expect(screen.queryByRole("button", { name: "Retry" })).toBeInTheDocument();
    expect(screen.getByTestId("transcript-status-pill")).not.toHaveTextContent(
      "Retrying",
    );
    expect(onRetryGeneration).not.toHaveBeenCalled();
  });
});

describe("ErrorMessageCard row mapping", () => {
  test("labels unclassified errors with the provider/model target", () => {
    const error: ErrorMessage = {
      role: "error",
      content: "openai/gpt-5 returned 429: usage limit reached",
    };
    render(<ErrorMessageCard errors={[error]} />);
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Error");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "openai/gpt-5",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent("429");
  });
});
