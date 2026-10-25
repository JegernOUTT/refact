import { describe, expect, test, vi } from "vitest";
import { screen } from "@testing-library/react";

import { render, waitFor } from "../../utils/test-utils";
import { ErrorCalloutView } from "./Callout";

describe("ErrorCalloutView", () => {
  test("keeps the retry label and click handler for retryable errors", async () => {
    const onClick = vi.fn();
    const { user } = render(
      <ErrorCalloutView onClick={onClick}>Request failed</ErrorCalloutView>,
    );

    expect(screen.getByText("Click to retry")).toBeInTheDocument();
    expect(screen.queryByText("Click to close")).not.toBeInTheDocument();

    await user.click(screen.getByText(/Request failed/));
    await waitFor(() => expect(onClick).toHaveBeenCalledTimes(1));
  });

  test("labels dismiss-only errors honestly and keeps the detail", async () => {
    const onClick = vi.fn();
    const detail = "Context rebuild failed: summary model returned no content";
    const { user } = render(
      <ErrorCalloutView onClick={onClick} preventRetry>
        {detail}
      </ErrorCalloutView>,
    );

    expect(screen.getByText("Click to close")).toBeInTheDocument();
    expect(screen.queryByText("Click to retry")).not.toBeInTheDocument();
    expect(screen.getByText(detail, { exact: false })).toBeInTheDocument();

    await user.click(screen.getByText(/Context rebuild failed/));
    await waitFor(() => expect(onClick).toHaveBeenCalledTimes(1));
  });

  test("auth errors show provider guidance instead of retry or close labels", async () => {
    const onClick = vi.fn();
    const { user } = render(
      <ErrorCalloutView onClick={onClick} isAuthError preventRetry>
        Unauthorized
      </ErrorCalloutView>,
    );

    expect(
      screen.getByText(
        "Check your provider configuration, API key, and network access.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Click to retry")).not.toBeInTheDocument();
    expect(screen.queryByText("Click to close")).not.toBeInTheDocument();

    await user.click(screen.getByText(/Unauthorized/));
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});
