import { Theme } from "@radix-ui/themes";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { CompactToolDetails } from "./CompactToolDetails";
import { COMPACT_VALUE_CHARS } from "./compactPresentation";
import { PagedText } from "./PagedText";

describe("CompactToolDetails", () => {
  it("shows at most three facts and explicit navigation without repeated chrome", async () => {
    const onOpen = vi.fn();
    const user = userEvent.setup();
    const { container } = render(
      <Theme>
        <CompactToolDetails
          facts={[
            { label: "Task", value: "Check the build" },
            { label: "Target", value: "Visible child title" },
            { label: "Count", value: "0" },
            { label: "Fourth", value: "Only in full body" },
          ]}
          navigation={<button onClick={onOpen}>Open child trajectory</button>}
        >
          <div>Full existing report</div>
        </CompactToolDetails>
      </Theme>,
    );
    expect(container.querySelectorAll("dt")).toHaveLength(3);
    expect(container.querySelectorAll("dd")).toHaveLength(3);
    expect(screen.getByText("0")).toBeVisible();
    expect(screen.queryByText("Fourth")).not.toBeInTheDocument();
    expect(screen.queryByRole("heading")).not.toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Open child trajectory" }),
    );
    expect(onOpen).toHaveBeenCalledOnce();
    expect(
      screen.getByRole("button", { name: "More details" }),
    ).toHaveAttribute("aria-expanded", "false");
    const fullReport = screen.getByText("Full existing report");
    expect(fullReport.closest("[inert]")).not.toBeNull();
  });

  it("starts the full body closed and inert, with keyboard toggle and focus isolation", async () => {
    const onAction = vi.fn();
    const user = userEvent.setup();
    const { container } = render(
      <Theme>
        <CompactToolDetails facts={[]} detailsLabel="Technical details">
          <p role="alert">Critical warning and diagnostic evidence</p>
          <button onClick={onAction}>Existing decision action</button>
        </CompactToolDetails>
        <button>After details</button>
      </Theme>,
    );
    const toggle = screen.getByRole("button", { name: "Technical details" });
    const body = container.querySelector("[inert]");
    expect(body).toHaveAttribute("id", toggle.getAttribute("aria-controls"));
    expect(body).toHaveAttribute("data-open", "false");
    expect(
      screen.getByRole("button", { name: "Existing decision action" }),
    ).toHaveAttribute("tabindex", "-1");
    await user.tab();
    expect(toggle).toHaveFocus();
    await user.tab();
    expect(screen.getByRole("button", { name: "After details" })).toHaveFocus();
    expect(onAction).not.toHaveBeenCalled();
    toggle.focus();
    await user.keyboard("{Enter}");
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(body).not.toHaveAttribute("inert");
    expect(body).toHaveAttribute("data-open", "true");
    expect(screen.getByRole("alert")).toBeVisible();
    await user.tab();
    expect(
      screen.getByRole("button", { name: "Existing decision action" }),
    ).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onAction).toHaveBeenCalledOnce();
    toggle.focus();
    await user.keyboard(" ");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(body).toHaveAttribute("inert");
    expect(screen.getByRole("alert").closest("[inert]")).toBe(body);
  });

  it("escapes hostile facts, bounds Unicode, and retains the full bounded renderer", async () => {
    const user = userEvent.setup();
    const hostile = '<img src=x onerror="alert(1)"><script>bad()</script>';
    const fullText = `${hostile}${"🧪".repeat(6_000)}END OF FULL RESULT`;
    const { container } = render(
      <Theme>
        <CompactToolDetails
          facts={[
            { label: hostile, value: hostile },
            { label: "Unicode", value: ` \n${"🧪".repeat(1_000)}\n ` },
            { label: "Absent", value: " \n " },
          ]}
        >
          <PagedText label="Full response" body={fullText} />
        </CompactToolDetails>
      </Theme>,
    );
    expect(container.querySelector("img, script")).toBeNull();
    expect(container.querySelector("dd")?.textContent).toBe(hostile);
    const values = container.querySelectorAll("dd");
    expect(values).toHaveLength(2);
    expect(Array.from(values[1].textContent ?? "")).toHaveLength(
      COMPACT_VALUE_CHARS,
    );
    expect(values[1].textContent).toBe(
      `${"🧪".repeat(COMPACT_VALUE_CHARS - 1)}…`,
    );
    expect(
      screen
        .getByRole("navigation", { name: "Full response pages" })
        .closest("[inert]"),
    ).not.toBeNull();
    await user.click(screen.getByRole("button", { name: "More details" }));
    expect(
      screen.getByLabelText("Full response content").textContent,
    ).toContain(hostile);
    while (
      !screen
        .getByLabelText("Full response content")
        .textContent?.includes("END OF FULL RESULT")
    ) {
      await user.click(screen.getByRole("button", { name: "Next section" }));
    }
    expect(container.querySelector("img, script")).toBeNull();
    await user.click(screen.getByRole("button", { name: "More details" }));
    await user.click(screen.getByRole("button", { name: "More details" }));
    expect(
      screen.getByLabelText("Full response content").textContent,
    ).toContain("END OF FULL RESULT");
  });

  it("keeps disclosure state independent and unchanged by updated facts", async () => {
    const user = userEvent.setup();
    const view = (value: string) => (
      <Theme>
        <CompactToolDetails facts={[{ label: "State", value }]}>
          <p>First body</p>
        </CompactToolDetails>
        <CompactToolDetails facts={[]}>
          <p>Second body</p>
        </CompactToolDetails>
      </Theme>
    );
    const { rerender } = render(view("Pending"));
    const toggles = screen.getAllByRole("button", { name: "More details" });
    expect(toggles[0].getAttribute("aria-controls")).not.toBe(
      toggles[1].getAttribute("aria-controls"),
    );
    rerender(view("Request failed"));
    expect(toggles[0]).toHaveAttribute("aria-expanded", "false");
    expect(toggles[1]).toHaveAttribute("aria-expanded", "false");
    await user.click(toggles[0]);
    rerender(view("Response received"));
    expect(toggles[0]).toHaveAttribute("aria-expanded", "true");
    expect(toggles[1]).toHaveAttribute("aria-expanded", "false");
    expect(screen.getByText("First body")).toBeVisible();
    expect(screen.getByText("Second body").closest("[inert]")).not.toBeNull();
  });

  it("omits empty facts, navigation, and disclosure when no body is supplied", () => {
    const { container } = render(<CompactToolDetails facts={[]} />);
    expect(container.querySelector("dl")).toBeNull();
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
