import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { act } from "react-dom/test-utils";
import { server } from "../../../utils/mockServer";
import { workspaceExample } from "../../../services/refact/pythonWorkspace.fixture";
import { PythonWorkspacePanel } from "./PythonWorkspacePanel";
import * as openFileHook from "../../../hooks/useOpenFileInApp";

const SECTIONS_KEY = "refact.pythonWorkspace.sections";

function serve(patch: Record<string, unknown> = {}) {
  server.use(
    http.get("*/v1/python/workspace", () =>
      HttpResponse.json({ ...workspaceExample, ...patch }),
    ),
  );
}

function variables(count: number) {
  return Array.from({ length: count }, (_, index) => ({
    name: `var_${String(index).padStart(3, "0")}`,
    type_name: "int",
    preview: String(index),
  }));
}

const runsSection = () => screen.findByRole("region", { name: "Recent runs" });

beforeEach(() => {
  window.localStorage.removeItem(SECTIONS_KEY);
  serve();
});
afterEach(() => {
  vi.restoreAllMocks();
  document.body.querySelectorAll("[data-python-run-id]").forEach((node) => {
    node.remove();
  });
});

describe("PythonWorkspacePanel", () => {
  it("orders sections Session, Recent runs, Artifacts, Variables and shows storage", async () => {
    render(<PythonWorkspacePanel chatId="chat-a" />);
    await runsSection();
    const names = screen
      .getAllByRole("region")
      .map((node) => node.getAttribute("aria-label"))
      .filter((name) => name !== "Python workspace");
    expect(names).toEqual(["Session", "Recent runs", "Artifacts", "Variables"]);
    expect(
      screen.getByRole("button", { name: "Recent runs · 1" }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Storage · 2 KB of 4 KB")).toBeInTheDocument();
    expect(
      screen.getByRole("progressbar", { name: "Workspace storage usage" }),
    ).toHaveAttribute("value", "2048");
  });
  it.each([
    ["completed", "Completed"],
    ["failed", "Failed"],
    ["cancelled", "Stopped"],
    ["finished", "Completed"],
  ])(
    "keeps terminal %s status without a finish timestamp",
    async (status, label) => {
      serve({
        runs: [{ ...workspaceExample.runs[0], status, finished_at_ms: null }],
      });
      render(<PythonWorkspacePanel chatId="chat-a" />);
      const section = await runsSection();
      expect(section).toHaveTextContent(
        new RegExp(
          `${label} · duration unavailable · .+ · 2 calls · background`,
        ),
      );
      expect(section).not.toHaveTextContent("Running");
    },
  );
  it("shows a live run as Background and offers it from the session", async () => {
    serve({
      session: { ...workspaceExample.session, status: "background" },
      runs: [
        {
          ...workspaceExample.runs[0],
          status: "background",
          finished_at_ms: null,
          description: "Wait for a slow job",
        },
      ],
    });
    render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    expect(section).toHaveTextContent(/Background · .+ · 2 calls/);
    expect(section).not.toHaveTextContent("· background");
    const session = screen.getByRole("region", { name: "Session" });
    expect(session).toHaveTextContent("Wait for a slow job");
    expect(
      within(session).getByRole("button", { name: "Show in chat" }),
    ).toBeInTheDocument();
  });
  it("titles runs by description, then code preview, then ID", async () => {
    serve({
      runs: [
        {
          ...workspaceExample.runs[0],
          run_id: "a",
          started_at_ms: 3,
          description: "Count TODOs",
        },
        {
          ...workspaceExample.runs[0],
          run_id: "b",
          started_at_ms: 2,
          code_preview: "x = 1",
        },
        { ...workspaceExample.runs[0], run_id: "c", started_at_ms: 1 },
      ],
    });
    render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    const rows = within(section).getAllByRole("listitem");
    expect(
      rows.map((row) => row.firstElementChild?.firstChild?.textContent),
    ).toEqual(["Count TODOs", "x = 1", "Run c"]);
  });
  it("shows five runs newest first with Show all", async () => {
    serve({
      runs: Array.from({ length: 7 }, (_, index) => ({
        ...workspaceExample.runs[0],
        run_id: `run-${index}`,
        started_at_ms: 1700000000000 + index,
      })),
    });
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    expect(within(section).getAllByRole("listitem")).toHaveLength(5);
    expect(within(section).getAllByRole("listitem")[0]).toHaveTextContent(
      "Run run-6",
    );
    await view.user.click(
      within(section).getByRole("button", { name: "Show all 7 runs" }),
    );
    expect(within(section).getAllByRole("listitem")).toHaveLength(7);
  });
  it("makes the run row Show in chat only when its card is rendered", async () => {
    const scroll = vi.fn();
    const cardNode = document.createElement("section");
    cardNode.setAttribute("data-python-run-id", "run-one");
    cardNode.scrollIntoView = scroll;
    document.body.appendChild(cardNode);
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    const row = within(section).getByTitle("Show in chat");
    expect(row).not.toHaveAttribute("aria-disabled");
    await view.user.click(row);
    expect(scroll).toHaveBeenCalled();
    cardNode.remove();
  });
  it("offers only usable run actions in the menu", async () => {
    const openFile = vi.fn();
    vi.spyOn(openFileHook, "useOpenFileInApp").mockReturnValue({
      openFile,
      canOpen: true,
    });
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    expect(
      within(section).getByTitle("This run's card is not loaded in the chat"),
    ).toHaveAttribute("aria-disabled", "true");
    await view.user.click(
      within(section).getByRole("button", { name: /^Actions for/ }),
    );
    await view.user.click(
      await screen.findByRole("menuitem", { name: "Open code" }),
    );
    expect(openFile).toHaveBeenCalledWith({
      path: "/python/runs/run-one/code.py",
    });
  });
  it("hides open actions when the host cannot open files", async () => {
    vi.spyOn(openFileHook, "useOpenFileInApp").mockReturnValue({
      openFile: vi.fn(),
      canOpen: false,
    });
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    await view.user.click(
      within(section).getByRole("button", { name: /^Actions for/ }),
    );
    expect(
      await screen.findByRole("menuitem", { name: "Copy run ID" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Open code" })).toBeNull();
    expect(screen.queryByRole("menuitem", { name: "Open output" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open chart.png" })).toBeNull();
  });
  it("shows artifacts basename first and opens them", async () => {
    const openFile = vi.fn();
    vi.spyOn(openFileHook, "useOpenFileInApp").mockReturnValue({
      openFile,
      canOpen: true,
    });
    serve({
      artifacts: [
        {
          ...workspaceExample.artifacts[0],
          path: "/python/artifacts/charts/chart.png",
          rel_path: "charts/chart.png",
        },
      ],
    });
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const artifacts = await screen.findByRole("region", { name: "Artifacts" });
    expect(artifacts).toHaveTextContent("chart.pngcharts1 KB");
    await view.user.click(
      within(artifacts).getByRole("button", { name: "Open chart.png" }),
    );
    expect(openFile).toHaveBeenCalledWith({
      path: "/python/artifacts/charts/chart.png",
    });
  });
  it("renders variables as single-line rows with one tab stop each", async () => {
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await screen.findByRole("region", { name: "Variables" });
    const row = within(section).getByRole("button", { name: /answer/ });
    expect(row).toHaveTextContent("answerint42");
    expect(section.querySelectorAll("[tabindex='0']")).toHaveLength(0);
    expect(within(section).getAllByRole("button")).toHaveLength(2);
    await view.user.click(row);
    expect(row).toHaveAttribute("aria-expanded", "true");
    expect(section.querySelector("pre")).toHaveTextContent("42");
  });
  it("collapses many variables, filters loaded ones and explains the cap", async () => {
    serve({
      session: {
        ...workspaceExample.session,
        variables: variables(200),
        variables_truncated: true,
        variables_total: 230,
      },
    });
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const toggle = await screen.findByRole("button", {
      name: "Variables · 230",
    });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await view.user.click(toggle);
    const section = screen.getByRole("region", { name: "Variables" });
    expect(section).toHaveTextContent(
      "200 of 230 variables loaded. Filtering searches loaded variables only.",
    );
    expect(section.querySelectorAll("li")).toHaveLength(12);
    await view.user.type(
      within(section).getByRole("searchbox", { name: "Filter variables" }),
      "var_1",
    );
    expect(section.querySelectorAll("li")).toHaveLength(12);
    await view.user.click(
      within(section).getByRole("button", {
        name: "Show all 100 loaded variables",
      }),
    );
    expect(section.querySelectorAll("li")).toHaveLength(100);
  });
  it("remembers section state", async () => {
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    await view.user.click(
      await screen.findByRole("button", { name: "Artifacts · 1" }),
    );
    view.unmount();
    render(<PythonWorkspacePanel chatId="chat-a" />);
    expect(
      await screen.findByRole("button", { name: "Artifacts · 1" }),
    ).toHaveAttribute("aria-expanded", "false");
  });
  it("labels the session from what the engine reports", async () => {
    const cases: [Record<string, unknown>, string][] = [
      [{ live: true }, "Live session"],
      [{ live: true, last_run_restored: true }, "Restored from disk"],
      [{ live: false, state_persisted: true }, "Saved on disk, not loaded"],
      [{ live: false, state_persisted: false }, "No saved state"],
    ];
    for (const [session, label] of cases) {
      serve({ session: { ...workspaceExample.session, ...session } });
      const view = render(<PythonWorkspacePanel chatId="chat-a" />);
      expect(await screen.findByText(label)).toBeInTheDocument();
      view.unmount();
    }
  });
  it("refreshes on demand and once per window focus", async () => {
    const request = vi.fn(() => HttpResponse.json(workspaceExample));
    server.use(http.get("*/v1/python/workspace", request));
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    await runsSection();
    await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
    await view.user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    window.dispatchEvent(new Event("focus"));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(3));
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(request).toHaveBeenCalledTimes(3);
  });
  it("ticks relative times once a minute without refetching", async () => {
    const request = vi.fn(() => HttpResponse.json(workspaceExample));
    server.use(http.get("*/v1/python/workspace", request));
    let now = 1700000000000 + 60_000;
    const clock = vi.spyOn(Date, "now").mockImplementation(() => now);
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      render(<PythonWorkspacePanel chatId="chat-a" />);
      const section = await runsSection();
      const time = section.querySelector("time");
      expect(time).toHaveTextContent("1m ago");
      expect(time?.getAttribute("title")).toMatch(/2023/);
      now += 2 * 60_000;
      act(() => {
        vi.advanceTimersByTime(60_000);
      });
      expect(section.querySelector("time")).toHaveTextContent("3m ago");
      expect(request).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
      clock.mockRestore();
    }
  });
  it("shows the empty state", async () => {
    serve({ exists: false });
    render(<PythonWorkspacePanel chatId="chat-a" />);
    expect(await screen.findByText("No script runs yet")).toBeInTheDocument();
    expect(
      screen.getByText(
        "This chat's session, runs and artifacts will appear here.",
      ),
    ).toBeInTheDocument();
  });
  it("shows no focused chat", () => {
    render(<PythonWorkspacePanel chatId={null} />);
    expect(
      screen.getByText("Focus a chat to see its script workspace"),
    ).toBeInTheDocument();
  });
  it("shows errors with Try again", async () => {
    server.use(
      http.get(
        "*/v1/python/workspace",
        () => new HttpResponse(null, { status: 500 }),
      ),
    );
    render(<PythonWorkspacePanel chatId="chat-a" />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't load workspace");
    expect(
      within(alert).getByRole("button", { name: "Try again" }),
    ).toBeInTheDocument();
  });
  it("keeps the filter while it has text after the list shrinks", async () => {
    serve({
      session: { ...workspaceExample.session, variables: variables(20) },
    });
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    await view.user.click(
      await screen.findByRole("button", { name: "Variables · 20" }),
    );
    const section = screen.getByRole("region", { name: "Variables" });
    await view.user.type(
      within(section).getByRole("searchbox", { name: "Filter variables" }),
      "var_01",
    );
    serve({
      session: { ...workspaceExample.session, variables: variables(5) },
    });
    await view.user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Variables · 5" }),
      ).toBeInTheDocument(),
    );
    const field = within(
      screen.getByRole("region", { name: "Variables" }),
    ).getByRole("searchbox", { name: "Filter variables" });
    await view.user.clear(field);
    expect(
      within(screen.getByRole("region", { name: "Variables" })).queryByRole(
        "searchbox",
      ),
    ).toBeNull();
    expect(
      screen.getByRole("region", { name: "Variables" }).querySelectorAll("li"),
    ).toHaveLength(5);
  });
  it("enables Show in chat for a card that appears after the panel mounted", async () => {
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    const row = within(section).getByTitle(
      "This run's card is not loaded in the chat",
    );
    expect(row).toHaveAttribute("aria-disabled", "true");
    const cardNode = document.createElement("section");
    cardNode.setAttribute("data-python-run-id", "run-one");
    const scroll = vi.fn();
    cardNode.scrollIntoView = scroll;
    document.body.appendChild(cardNode);
    await view.user.hover(row);
    expect(row).not.toHaveAttribute("aria-disabled");
    await view.user.click(row);
    expect(scroll).toHaveBeenCalled();
  });
  it("titles a run without description by its preview, like the card", async () => {
    serve({
      runs: [{ ...workspaceExample.runs[0], code_preview: "# Count TODOs" }],
    });
    render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    expect(section).toHaveTextContent("Count TODOs");
    expect(section).not.toHaveTextContent("# Count");
  });
  it("hides open actions for a run ID that is not a plain name", async () => {
    vi.spyOn(openFileHook, "useOpenFileInApp").mockReturnValue({
      openFile: vi.fn(),
      canOpen: true,
    });
    serve({ runs: [{ ...workspaceExample.runs[0], run_id: "../../x" }] });
    const view = render(<PythonWorkspacePanel chatId="chat-a" />);
    const section = await runsSection();
    await view.user.click(
      within(section).getByRole("button", { name: /^Actions for/ }),
    );
    expect(
      await screen.findByRole("menuitem", { name: "Copy run ID" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: "Open code" })).toBeNull();
  });
  describe("displays", () => {
    const displays = [
      {
        index: 1,
        kind: "table",
        title: "Slowest tests",
        columns: ["name", "s"],
        rows: [["a", 1]],
        rows_total: 40,
        truncated: true,
        path: "/python/runs/run-one/displays/1.json",
        run_id: "run-one",
        started_at_ms: Date.now() - 120_000,
      },
      {
        index: 2,
        kind: "markdown",
        text: "## Notes",
        truncated: false,
        path: "/python/runs/run-one/displays/2.json",
        run_id: "run-one",
        started_at_ms: Date.now() - 120_000,
      },
    ];
    it("is hidden without displays and sits between Recent runs and Artifacts", async () => {
      const view = render(<PythonWorkspacePanel chatId="chat-a" />);
      await runsSection();
      expect(screen.queryByRole("region", { name: "Displays" })).toBeNull();
      view.unmount();
      serve({ displays });
      render(<PythonWorkspacePanel chatId="chat-a" />);
      await screen.findByRole("region", { name: "Displays" });
      const names = screen
        .getAllByRole("region")
        .map((node) => node.getAttribute("aria-label"))
        .filter((name) =>
          ["Recent runs", "Displays", "Artifacts"].includes(name ?? ""),
        );
      expect(names).toEqual(["Recent runs", "Displays", "Artifacts"]);
    });
    it("lists compact rows that expand in place with Show in chat and Open full data", async () => {
      const openFile = vi.fn();
      vi.spyOn(openFileHook, "useOpenFileInApp").mockReturnValue({
        openFile,
        canOpen: true,
      });
      serve({ displays });
      const view = render(<PythonWorkspacePanel chatId="chat-a" />);
      const section = await screen.findByRole("region", { name: "Displays" });
      const row = within(section).getByRole("button", {
        name: /^Slowest tests 40 rows × 2 columns · 2m ago$/,
      });
      expect(row).toHaveAttribute("aria-expanded", "false");
      await view.user.click(row);
      expect(row).toHaveAttribute("aria-expanded", "true");
      expect(
        within(section).getByRole("region", { name: "Slowest tests table" }),
      ).toBeInTheDocument();
      expect(within(section).getByText("Showing 1 of 40 rows")).toBeVisible();
      expect(
        within(section).getByRole("button", { name: "Show in chat" }),
      ).toHaveAttribute("aria-disabled", "true");
      await view.user.click(
        within(section).getByRole("button", {
          name: "Open full data of Slowest tests",
        }),
      );
      expect(openFile).toHaveBeenCalledWith({
        path: "/python/runs/run-one/displays/1.json",
      });
    });
    it("remembers the Displays section state", async () => {
      serve({ displays });
      const view = render(<PythonWorkspacePanel chatId="chat-a" />);
      await view.user.click(
        await screen.findByRole("button", { name: "Displays · 2" }),
      );
      view.unmount();
      render(<PythonWorkspacePanel chatId="chat-a" />);
      expect(
        await screen.findByRole("button", { name: "Displays · 2" }),
      ).toHaveAttribute("aria-expanded", "false");
    });
    it("adds model calls and displays to the run meta", async () => {
      serve({
        runs: [
          {
            ...workspaceExample.runs[0],
            status: "completed",
            calls_total: 14,
            llm_calls: 12,
            displays_total: 3,
            engine_changes: 2,
          },
        ],
      });
      const section = await (async () => {
        render(<PythonWorkspacePanel chatId="chat-a" />);
        return runsSection();
      })();
      expect(section).toHaveTextContent(
        /2 calls · 12 model calls · 3 displays · 2 engine changes · background/,
      );
    });
  });
});
