vi.hoisted(() => vi.resetModules());
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "../../utils/test-utils";
import { pythonCall } from "../../__tests__/pythonFixtures";
import { NestedToolCalls } from "./NestedToolCalls";
import type { DiffMessage } from "../../services/refact/types";

const observed = vi.hoisted(() => ({ diffs: [] as DiffMessage[][] }));
vi.mock("./DiffContent", () => ({
  GroupedDiffs: ({ diffs }: { diffs: DiffMessage[] }) => {
    observed.diffs.push(diffs);
    return null;
  },
}));

describe("NestedToolCalls", () => {
  it("preserves derived diff arrays when unchanged calls rerender", () => {
    const calls = [
      {
        ...pythonCall,
        diffs: [
          {
            file_name: "/file",
            file_action: "edit",
            line1: 1,
            line2: 1,
            lines_remove: "before\n",
            lines_add: "after\n",
          },
        ],
      },
    ];
    const view = render(<NestedToolCalls calls={calls} />);
    fireEvent.click(screen.getByRole("button", { name: /shell/ }));
    const first = observed.diffs.at(-1);
    expect(first).toBeDefined();
    view.rerender(<NestedToolCalls calls={calls} />);
    expect(observed.diffs.at(-1)).toBe(first);
  });
  it("orders rows by seq and shows a one-line summary, not raw JSON", () => {
    const { container } = render(
      <NestedToolCalls
        root="/tmp/project"
        calls={[
          {
            ...pythonCall,
            seq: 2,
            call_id: "b",
            tool: "cat",
            status: "ok",
            duration_ms: 1500,
            approval: "allowed",
            args_preview: JSON.stringify({ paths: "/tmp/project/src/a.py" }),
            summary: "Paths found:",
          },
          {
            ...pythonCall,
            seq: 1,
            call_id: "a",
            tool: "shell",
            status: "ok",
            duration_ms: 20,
            args_preview: JSON.stringify({ command: "ls -la", workdir: "/x" }),
          },
        ]}
      />,
    );
    const rows = [...container.querySelectorAll("li")];
    expect(rows.map((row) => row.querySelector("button")?.textContent)).toEqual(
      [
        expect.stringContaining("shellls -la20 ms"),
        expect.stringContaining("catsrc/a.pyAllowed once1.5 s"),
      ],
    );
    expect(container.textContent).not.toContain("{");
    expect(screen.queryByText("Not needed")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /cat/ }));
    expect(screen.getByText("paths")).toBeInTheDocument();
    expect(screen.getByText("Paths found:")).toBeInTheDocument();
  });
  it("marks the call that waits for approval", () => {
    render(
      <NestedToolCalls
        calls={[{ ...pythonCall, call_id: "c" }]}
        waitingCallIds={new Set(["c"])}
      />,
    );
    expect(screen.getByText("Waiting for your approval")).toBeInTheDocument();
  });
  it("keeps the newest twenty calls and expands earlier ones", () => {
    const calls = Array.from({ length: 30 }, (_, i) => ({
      ...pythonCall,
      seq: i + 1,
      call_id: `c${i + 1}`,
      status: "ok" as const,
      args_preview: JSON.stringify({ command: `step ${i + 1}` }),
    }));
    render(<NestedToolCalls calls={calls} />);
    expect(screen.queryByText("step 1")).toBeNull();
    expect(screen.getByText("step 30")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Show 10 earlier calls" }),
    );
    expect(screen.getByText("step 1")).toBeInTheDocument();
  });
  it("summarizes a call by its description and keeps the command in details", () => {
    render(
      <NestedToolCalls
        calls={[
          {
            ...pythonCall,
            args_preview: JSON.stringify({
              command: "sleep 8 && echo done",
              description: "Wait for a slow command",
            }),
          },
        ]}
      />,
    );
    expect(screen.getByText("Wait for a slow command")).toBeInTheDocument();
    expect(screen.queryByText("sleep 8 && echo done")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /shell/ }));
    expect(screen.getByText("sleep 8 && echo done")).toBeInTheDocument();
  });
  it("summarises an engine call by its result, not a generic argument", () => {
    render(
      <NestedToolCalls
        calls={[
          {
            ...pythonCall,
            tool: "engine.settings.set",
            args_preview: '{"key":"autonomy.max_steps","value":40}',
            status: "ok",
            approval: "allowed",
            summary: "autonomy.max_steps: 25 → 40",
          },
        ]}
      />,
    );
    const row = screen.getByRole("button", { name: /engine\.settings\.set/ });
    expect(row).toHaveTextContent("autonomy.max_steps: 25 → 40");
    expect(row).toHaveTextContent("Allowed once");
    fireEvent.click(row);
    expect(screen.getByText("settings.set")).toBeInTheDocument();
  });
  it("labels a refused engine call neutrally, not as the user's answer", () => {
    render(
      <NestedToolCalls
        calls={[
          {
            ...pythonCall,
            tool: "engine.settings.set",
            status: "denied",
            approval: "denied",
            summary: "engine changes are switched off for scripts",
          },
        ]}
      />,
    );
    const row = screen.getByRole("button", { name: /engine\.settings\.set/ });
    expect(row).toHaveTextContent("Denied");
    expect(row.textContent).not.toMatch(/by you/);
    expect(row).toHaveTextContent("engine changes are switched off");
  });
  it("shows invisible characters of an engine summary as visible marks", () => {
    render(
      <NestedToolCalls
        calls={[
          {
            ...pythonCall,
            tool: "engine.settings.set",
            status: "ok",
            approval: "allowed",
            summary: "title: a → b\u202Ec",
          },
        ]}
      />,
    );
    const row = screen.getByRole("button", { name: /engine\.settings\.set/ });
    expect(row.textContent).not.toContain("\u202E");
    expect(row).toHaveTextContent("⟨U+202E⟩");
    fireEvent.click(row);
    expect(document.body.textContent).not.toContain("\u202E");
  });
});
