import React from "react";
import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import type {
  ProviderListItem,
  ProviderQuotaSnapshot,
} from "../../services/refact/providers";
import {
  ProviderUsageIndicatorContent,
  SnapshotRows,
} from "./ProviderUsageIndicator";
import { resolveSelectedProvider } from "./providerUsageIndicator";

vi.mock("../LongTailPrimitives", () => {
  const Flex = React.forwardRef<
    HTMLDivElement,
    React.HTMLAttributes<HTMLDivElement>
  >(({ children, ...props }, ref) => (
    <div ref={ref} {...props}>
      {children}
    </div>
  ));
  Flex.displayName = "Flex";
  const Text: React.FC<React.HTMLAttributes<HTMLSpanElement>> = ({
    children,
    ...props
  }) => <span {...props}>{children}</span>;
  const Root: React.FC<React.PropsWithChildren> = ({ children }) => children;
  const Trigger: React.FC<React.PropsWithChildren> = ({ children }) => children;
  const Content: React.FC<React.PropsWithChildren> = ({ children }) => children;
  const ScrollArea: React.FC<React.PropsWithChildren> = ({ children }) =>
    children;
  return { Flex, Text, ScrollArea, HoverCard: { Root, Trigger, Content } };
});

const providers: ProviderListItem[] = [
  {
    name: "account-a",
    base_provider: "custom-provider",
    display_name: "Account A",
    enabled: true,
    readonly: false,
    has_credentials: true,
    status: "active",
    model_count: 1,
  },
  {
    name: "account-b",
    base_provider: "another-provider",
    display_name: "Account B",
    enabled: true,
    readonly: false,
    has_credentials: true,
    status: "active",
    model_count: 1,
  },
];

function snapshot(
  overrides: Partial<ProviderQuotaSnapshot> = {},
): ProviderQuotaSnapshot {
  return {
    provider_name: "account-b",
    base_provider: "another-provider",
    source: "provider-api",
    available: true,
    fetched_at: "2025-01-01T00:00:00Z",
    stale: false,
    windows: [],
    facts: [],
    ...overrides,
  };
}

function renderIndicator(
  quota: Partial<{
    data: { quota: ProviderQuotaSnapshot };
    isError: boolean;
    isLoading: boolean;
  }> = {},
) {
  return render(
    <ProviderUsageIndicatorContent
      displayName="Account B"
      providerName="account-b"
      quotaQuery={{
        isError: false,
        isLoading: false,
        ...quota,
      }}
    />,
  );
}

describe("ProviderUsageIndicator", () => {
  it("resolves the selected model to its exact configured provider", () => {
    expect(resolveSelectedProvider("account-b/model-x", providers)).toEqual(
      providers[1],
    );
    expect(resolveSelectedProvider("unknown/model-x", providers)).toBeNull();
  });

  it("renders normalized windows, facts, source, and plan data", () => {
    render(
      <SnapshotRows
        snapshot={snapshot({
          windows: [
            {
              id: "weekly",
              label: "Weekly messages",
              used_percent: 75,
              used: 750,
              limit: 1000,
              reset_after_seconds: 3600,
              window_seconds: 604800,
            },
          ],
          facts: [
            { id: "plan", label: "Plan", value: "Pro" },
            { id: "credits", label: "Credits", value: 12, unit: "USD" },
          ],
        })}
      />,
    );

    expect(
      screen.getByRole("progressbar", { name: "Weekly messages" }),
    ).toHaveAttribute("aria-valuenow", "75");
    expect(screen.getByText("750 / 1,000")).toBeInTheDocument();
    expect(screen.getByText("Pro")).toBeInTheDocument();
    expect(screen.getByText("12 USD")).toBeInTheDocument();
    expect(screen.getByText("provider-api")).toBeInTheDocument();
  });

  it("keeps the indicator visible for an unavailable snapshot", () => {
    render(
      <SnapshotRows
        snapshot={snapshot({ available: false, source: "not-supported" })}
      />,
    );

    expect(screen.getByText("Unavailable")).toBeInTheDocument();
    expect(screen.getByText("not-supported")).toBeInTheDocument();
  });

  it("labels available quota without a percentage as unknown", () => {
    renderIndicator({ data: { quota: snapshot({ available: true }) } });

    expect(screen.getByLabelText("Account B: Usage unknown")).toBeVisible();
  });

  it("shows a stale snapshot without hiding its data", () => {
    render(
      <SnapshotRows
        snapshot={snapshot({
          stale: true,
          facts: [{ id: "plan", label: "Plan", value: "Team" }],
        })}
      />,
    );

    expect(screen.getByText("Stale")).toBeInTheDocument();
    expect(screen.getByText("Team")).toBeInTheDocument();
  });

  it("shows an error carried by the snapshot", () => {
    render(<SnapshotRows snapshot={snapshot({ error: "Refresh failed" })} />);

    expect(screen.getByText("Refresh failed")).toBeInTheDocument();
  });

  it("shows loading, query error, and missing snapshot states", () => {
    const { rerender } = renderIndicator({ isLoading: true });
    expect(screen.getByLabelText("Account B: Loading quota")).toBeVisible();

    rerender(
      <ProviderUsageIndicatorContent
        displayName="Account B"
        providerName="account-b"
        quotaQuery={{ isError: true, isLoading: false }}
      />,
    );
    expect(screen.getByLabelText("Account B: Quota error")).toBeVisible();

    rerender(
      <ProviderUsageIndicatorContent
        displayName="Account B"
        providerName="account-b"
        quotaQuery={{ isError: false, isLoading: false }}
      />,
    );
    expect(screen.getByLabelText("Account B: Quota unavailable")).toBeVisible();
  });
});
