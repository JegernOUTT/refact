import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { render, screen, fireEvent, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { WorkPanel } from "./WorkPanel";
import { workFixture } from "../../../services/refact/workFixtures";
import { workApi } from "../../../services/refact/work";
import * as subscription from "../../../services/refact/workSubscription";

afterEach(() => vi.restoreAllMocks());

const renderPanel = () =>
  render(<WorkPanel />, {
    preloadedState: {
      config: {
        host: "ide",
        lspPort: 8001,
        themeProps: { appearance: "dark" },
      },
    },
  });

function settingsHandler() {
  server.use(
    http.get("*/v1/coordination-settings", () =>
      HttpResponse.json({ effective: {} }),
    ),
  );
}

describe("Work panel", () => {
  it("renders explicit unavailable state without empty success", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({ code: "scope_denied" }, { status: 403 }),
      ),
    );
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Work unavailable",
    );
    expect(screen.queryByText("No observed work")).not.toBeInTheDocument();
  });
  it("renders an honest empty projection", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({ ...workFixture, rows: [], total: 0, returned: 0 }),
      ),
    );
    renderPanel();
    expect(await screen.findByText("No observed work")).toBeInTheDocument();
    expect(
      screen.getByText(/does not mean files are safe/),
    ).toBeInTheDocument();
  });
  it("distinguishes parked, attention and historical work", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({
          ...workFixture,
          rows: [
            {
              ...workFixture.rows[0],
              state: "parked",
              attention: true,
              freshness: "stale",
            },
          ],
        }),
      ),
    );
    renderPanel();
    expect(await screen.findByText("Parked")).toBeInTheDocument();
    expect(screen.getByText("Needs attention")).toBeInTheDocument();
    expect(screen.getByText(/Stale/)).toBeInTheDocument();
    expect(screen.getByText(/File activity/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
  });

  it("renders a closed row instead of blanking the panel", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({
          ...workFixture,
          rows: [{ ...workFixture.rows[0], state: "closed" }],
        }),
      ),
    );
    renderPanel();
    expect(await screen.findByLabelText("Work state")).toHaveAttribute(
      "data-value",
      "closed",
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("reports zero, one and many waiting questions without inventing a count", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({
          ...workFixture,
          rows: [
            { ...workFixture.rows[0], open_questions: 0 },
            {
              ...workFixture.rows[0],
              key: { runtime: "trajectory", id: "one" },
              open_questions: 1,
            },
            {
              ...workFixture.rows[0],
              key: { runtime: "trajectory", id: "many" },
              open_questions: 4,
            },
            {
              ...workFixture.rows[0],
              key: { runtime: "trajectory", id: "silent" },
            },
          ],
          total: 4,
          returned: 4,
        }),
      ),
    );
    renderPanel();
    const questions = await screen.findAllByLabelText("Waiting questions");
    expect(questions.map((node) => node.textContent)).toEqual([
      "No waiting questions",
      "1 waiting question",
      "4 waiting questions",
      "Waiting questions: unknown",
    ]);
    expect(questions[3]).toHaveAttribute("data-value", "unknown");
  });

  it("names the admission class a queued row waits in, or says unknown", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({
          ...workFixture,
          rows: [
            { ...workFixture.rows[0], priority: "user_blocking" },
            {
              ...workFixture.rows[0],
              key: { runtime: "trajectory", id: "plain" },
            },
          ],
          total: 2,
          returned: 2,
        }),
      ),
    );
    renderPanel();
    const classes = await screen.findAllByLabelText("Admission class");
    expect(classes[0]).toHaveAttribute("data-value", "user_blocking");
    expect(classes[1]).toHaveAttribute("data-value", "unknown");
    expect(classes[1]).toHaveTextContent("unknown");
  });

  it("withholds the per-row signals of a withheld row rather than guessing them", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({
          ...workFixture,
          rows: [
            {
              ...workFixture.rows[0],
              title: undefined,
              disclosure: "withheld",
            },
          ],
        }),
      ),
    );
    renderPanel();
    await screen.findByText("Work withheld by privacy policy");
    expect(
      screen.queryByLabelText("Waiting questions"),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Admission class")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Work state")).toHaveAttribute(
      "data-value",
      "queued",
    );
  });

  it("renders one, many and paged rows with truthful totals", async () => {
    settingsHandler();
    const many = Array.from({ length: 3 }, (_, index) => ({
      ...workFixture.rows[0],
      key: { runtime: "trajectory" as const, id: `chat-${index}` },
      title: `Work ${index}`,
      state: index === 0 ? ("queued" as const) : ("running" as const),
    }));
    server.use(
      http.get("*/v1/work", ({ request }) =>
        HttpResponse.json(
          new URL(request.url).searchParams.get("cursor") === "page-2"
            ? {
                ...workFixture,
                rows: [many[2]],
                total: 3,
                returned: 1,
              }
            : {
                ...workFixture,
                rows: many.slice(0, 2),
                total: 3,
                returned: 2,
                next_cursor: "page-2",
              },
        ),
      ),
    );
    renderPanel();
    expect(await screen.findByText("Work 0")).toBeInTheDocument();
    expect(screen.getByText("Work 1")).toBeInTheDocument();
    expect(screen.queryByText("Work 2")).not.toBeInTheDocument();
    expect(screen.getByText(/3 observed/)).toBeInTheDocument();
    expect(screen.getByText("Queued")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(await screen.findByText("Work 2")).toBeInTheDocument();
    expect(screen.queryByText("Work 0")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next page" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "First page" }));
    expect(await screen.findByText("Work 0")).toBeInTheDocument();
  });

  it("renders nothing about overlap when the page reports none", async () => {
    settingsHandler();
    server.use(http.get("*/v1/work", () => HttpResponse.json(workFixture)));
    renderPanel();
    await screen.findByText("Board work");
    expect(screen.queryByLabelText("File overlap")).not.toBeInTheDocument();
    expect(screen.queryByText(/overlap/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/same physical file/i)).not.toBeInTheDocument();
  });

  it("marks a withheld row without leaking it and without inventing a count", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({
          ...workFixture,
          rows: [
            {
              ...workFixture.rows[0],
              title: undefined,
              disclosure: "withheld",
            },
            {
              ...workFixture.rows[0],
              key: { runtime: "trajectory", id: "visible" },
              title: "Visible work",
              disclosure: "full",
            },
          ],
          total: 2,
          returned: 2,
        }),
      ),
    );
    renderPanel();
    expect(
      await screen.findByText("Work withheld by privacy policy"),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText("Withheld by privacy policy"),
    ).toBeInTheDocument();
    expect(screen.getByText("Visible work")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Withheld work count"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText(/reduced by your privacy policy/),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/File activity/)).toBeInTheDocument();
  });

  it("reports the withheld count only when the payload supplies one", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({ ...workFixture, withheld_rows: 3 }),
      ),
    );
    renderPanel();
    expect(
      await screen.findByLabelText("Withheld work count"),
    ).toHaveTextContent("3 rows were reduced by your privacy policy.");
  });

  it("hides cached rows immediately after a visibility failure", async () => {
    server.use(
      http.get("*/v1/coordination-settings", () =>
        HttpResponse.json({
          effective: {
            work_stream_timeout_ms: 60_000,
            work_reconnect_max_ms: 10_000,
          },
        }),
      ),
      http.get("*/v1/work", () => HttpResponse.json(workFixture)),
      http.get(
        "*/v1/work/subscribe",
        () =>
          new HttpResponse(
            'event: work_error\ndata: {"code":"policy_unavailable"}\n\n',
            { headers: { "Content-Type": "text/event-stream" } },
          ),
      ),
    );
    const view = renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Work unavailable",
    );
    expect(screen.queryByText("Board work")).not.toBeInTheDocument();
    view.unmount();
    view.store.dispatch(workApi.util.resetApiState());
  });

  it("Refresh reconnects immediately and keeps old rows hidden until a fresh stream snapshot", async () => {
    const streams: subscription.WorkStreamOptions[] = [];
    let releaseRefresh: (() => void) | undefined;
    const refreshReady = new Promise<void>((resolve) => {
      releaseRefresh = resolve;
    });
    let requests = 0;
    const stopped = vi.fn();
    vi.spyOn(subscription, "subscribeToWork").mockImplementation(
      (_url, _token, options) => {
        streams.push(options);
        return stopped;
      },
    );
    server.use(
      http.get("*/v1/coordination-settings", () =>
        HttpResponse.json({
          effective: {
            work_stream_timeout_ms: 60_000,
            work_reconnect_max_ms: 60_000,
          },
        }),
      ),
      http.get("*/v1/work", async () => {
        requests += 1;
        if (requests > 1) await refreshReady;
        return HttpResponse.json(workFixture);
      }),
    );
    const view = renderPanel();
    await screen.findByText("Board work");
    await waitFor(() => expect(streams).toHaveLength(1));
    act(() => {
      streams[0].onError(
        "Work visibility changed or service became unavailable",
      );
    });
    await screen.findByRole("alert");
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(streams).toHaveLength(2));
    await waitFor(() => expect(requests).toBe(2));
    expect(screen.queryByText("Board work")).not.toBeInTheDocument();
    expect(stopped).toHaveBeenCalledOnce();
    act(() => {
      streams[1].onSnapshot({
        ...workFixture,
        seq: 71,
        rows: [{ ...workFixture.rows[0], title: "Fresh result" }],
      });
    });
    expect(await screen.findByText("Fresh result")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    releaseRefresh?.();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Refresh project work" }),
      ).toBeEnabled(),
    );
    expect(screen.getByText("Fresh result")).toBeInTheDocument();
    expect(screen.queryByText("Board work")).not.toBeInTheDocument();
    view.unmount();
    view.store.dispatch(workApi.util.resetApiState());
  });
});
