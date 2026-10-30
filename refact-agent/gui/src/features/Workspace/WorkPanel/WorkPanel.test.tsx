import { afterEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { WorkPanel } from "./WorkPanel";
import { workFixture } from "../../../services/refact/workFixtures";
import { setChatModel } from "../../Chat/Thread/actions";
import { selectCurrentThreadId } from "../../Chat/Thread/selectors";
import { updateConfig } from "../../Config/configSlice";
import { AgentWorkSection } from "./AgentWorkSection";
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
  it("keeps project-work selections on the generic callback", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work", () =>
        HttpResponse.json({
          ...workFixture,
          rows: [{ ...workFixture.rows[0], chat_id: "project-chat" }],
        }),
      ),
    );
    const onNavigate = vi.fn();
    const onAgentNavigate = vi.fn();
    const { user } = render(
      <AgentWorkSection
        chatId={null}
        onNavigate={onNavigate}
        onAgentNavigate={onAgentNavigate}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Project work" }));
    await user.click(
      await screen.findByRole("button", { name: "Open trajectory" }),
    );
    expect(onNavigate).toHaveBeenCalledWith("project-chat");
    expect(onAgentNavigate).not.toHaveBeenCalled();
  });

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
    expect(JSON.stringify(view.store.getState().workApi)).not.toContain(
      "Board work",
    );
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
        { kind: "visibility", data: { code: "policy_unavailable" } },
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

  it("rejects a REST completion dispatched before a stream denial until an explicit fresh request", async () => {
    const streams: subscription.WorkStreamOptions[] = [];
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    let requests = 0;
    vi.spyOn(subscription, "subscribeToWork").mockImplementation(
      (_url, _token, options) => {
        streams.push(options);
        return vi.fn();
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
        if (requests === 2) await pending;
        return HttpResponse.json(workFixture);
      }),
    );
    const view = renderPanel();
    await screen.findByText("Board work");
    await waitFor(() => expect(streams).toHaveLength(1));
    fireEvent.click(
      screen.getByRole("button", { name: "Refresh project work" }),
    );
    await waitFor(() => expect(requests).toBe(2));
    act(() =>
      streams[streams.length - 1].onError("denied", {
        kind: "http",
        status: 401,
      }),
    );
    await screen.findByRole("alert");
    release();
    await waitFor(() =>
      expect(
        view.store.dispatch(workApi.util.getRunningQueriesThunk()),
      ).toHaveLength(0),
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Authentication is required",
    );
    expect(JSON.stringify(view.store.getState().workApi)).not.toContain(
      "Board work",
    );
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("Board work")).toBeInTheDocument();
    expect(requests).toBe(3);
    view.unmount();
    view.store.dispatch(workApi.util.resetApiState());
  });

  it("keeps only same-scope transport failures readable and drops late callbacks after token and endpoint changes", async () => {
    const streams: subscription.WorkStreamOptions[] = [];
    const stopped = vi.fn();
    const requests: { url: string; authorization: string | null }[] = [];
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
      http.get("*/v1/work", ({ request }) => {
        requests.push({
          url: request.url,
          authorization: request.headers.get("Authorization"),
        });
        return HttpResponse.json(workFixture);
      }),
    );
    const view = renderPanel();
    await screen.findByText("Board work");
    await waitFor(() => expect(streams).toHaveLength(1));
    act(() => streams[0].onError("private raw failure", { kind: "transport" }));
    expect(screen.getByText("Board work")).toBeInTheDocument();
    expect(screen.getByText(/Showing a stale snapshot/)).toHaveTextContent(
      "Reconnecting",
    );
    expect(screen.queryByText("private raw failure")).not.toBeInTheDocument();
    act(() => {
      view.store.dispatch(
        updateConfig({
          lspUrl: "http://localhost:9123/project/new",
          apiKey: "new-token",
        }),
      );
    });
    await waitFor(() => expect(streams).toHaveLength(2));
    expect(stopped).toHaveBeenCalledOnce();
    act(() => {
      view.store.dispatch(
        setChatModel({
          model: "different/model",
          chatId: selectCurrentThreadId(view.store.getState()),
        }),
      );
    });
    await waitFor(() => expect(streams).toHaveLength(3));
    act(() =>
      streams[0].onSnapshot({
        ...workFixture,
        rows: [{ ...workFixture.rows[0], title: "Late private row" }],
      }),
    );
    expect(screen.queryByText("Late private row")).not.toBeInTheDocument();
    expect(requests[requests.length - 1]).toEqual({
      url: "http://localhost:9123/project/new/v1/work",
      authorization: "Bearer new-token",
    });
    act(() =>
      streams[2].onError("secret path /private", { kind: "http", status: 401 }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Authentication is required",
    );
    expect(screen.queryByText("Board work")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/Showing a stale snapshot/),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("secret path /private")).not.toBeInTheDocument();
    view.unmount();
  });

  it("refetches an expired cursor from the first page without repeating the expired request", async () => {
    settingsHandler();
    const cursors: (string | null)[] = [];
    server.use(
      http.get("*/v1/work", ({ request }) => {
        const cursor = new URL(request.url).searchParams.get("cursor");
        cursors.push(cursor);
        return cursor
          ? HttpResponse.json(
              { code: "cursor_expired", message: "private" },
              { status: 409 },
            )
          : HttpResponse.json({ ...workFixture, next_cursor: "expired" });
      }),
    );
    renderPanel();
    await screen.findByText("Board work");
    fireEvent.click(screen.getByRole("button", { name: "Next page" }));
    await waitFor(() => expect(cursors).toEqual([null, "expired", null]));
    expect(await screen.findByText("Board work")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps every exact dock label keyboard reachable and mounts only the selected view", async () => {
    const requests: string[] = [];
    server.use(
      http.get("*/v1/:surface", ({ request, params }) => {
        if (
          !["work", "attention", "hooks", "metrics"].includes(
            String(params.surface),
          )
        )
          return HttpResponse.json({});
        expect(request.headers.get("Authorization")).toBe("Bearer dock-token");
        requests.push(request.url);
        return HttpResponse.json(
          { code: "unauthorized", message: "/private/token" },
          { status: 401 },
        );
      }),
    );
    settingsHandler();
    const { user } = render(<AgentWorkSection chatId={null} />, {
      preloadedState: {
        config: {
          host: "ide",
          lspPort: 8001,
          lspUrl: "http://localhost:9124/project/dock",
          apiKey: "dock-token",
          themeProps: { appearance: "dark" },
        },
      },
    });
    const nav = screen.getByRole("group", { name: "Agent work scope" });
    const labels = [
      "This chat",
      "Project work",
      "Causality",
      "Attention",
      "Hooks",
      "Metrics",
    ];
    expect(
      within(nav)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(labels);
    expect(requests).toEqual([]);
    within(nav).getByRole("button", { name: "This chat" }).focus();
    for (const label of labels.slice(1)) {
      await user.tab();
      expect(within(nav).getByRole("button", { name: label })).toHaveFocus();
      await user.keyboard("{Enter}");
      expect(within(nav).getByRole("button", { name: label })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      if (label === "Causality") {
        expect(
          await screen.findByText("No event selected"),
        ).toBeInTheDocument();
      } else {
        expect(await screen.findByRole("alert")).toHaveTextContent(
          "Authentication is required",
        );
      }
      expect(screen.queryByText("/private/token")).not.toBeInTheDocument();
    }
    expect(requests.map((url) => new URL(url).pathname)).toEqual([
      "/project/dock/v1/work",
      "/project/dock/v1/attention",
      "/project/dock/v1/hooks",
      "/project/dock/v1/metrics",
    ]);
    expect(
      screen.queryByRole("region", { name: "Project work" }),
    ).not.toBeInTheDocument();
  });
});
