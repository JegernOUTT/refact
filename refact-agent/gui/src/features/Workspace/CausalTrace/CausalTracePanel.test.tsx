import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, fireEvent } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { CausalTracePanel } from "./CausalTracePanel";
import { causalPageFixture } from "../../../services/refact/causalFixtures";
import type {
  CausalEvent,
  CausalPage,
} from "../../../services/refact/causalTypes";

afterEach(() => vi.restoreAllMocks());

const renderPanel = () =>
  render(<CausalTracePanel />, {
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

function traceHandler(page: Record<string, unknown> | CausalPage) {
  server.use(http.get("*/v1/work/causal", () => HttpResponse.json(page)));
}

function trace(root = "decision:d") {
  renderPanel();
  fireEvent.change(screen.getByLabelText("Root event id"), {
    target: { value: root },
  });
  fireEvent.click(screen.getByRole("button", { name: "Trace" }));
}

const event = (index: number): CausalEvent => ({
  event_id: `event:${index}`,
  seq: index,
  kind: "delivery",
  scope: "chat",
  work: { runtime: "trajectory", id: "chat" },
  causes: [{ event: `event:${index - 1}` }],
  recorded_at_ms: 1_000 + index,
});

describe("Causal trace panel", () => {
  it("asks for a root before claiming anything about causality", async () => {
    settingsHandler();
    renderPanel();
    expect(await screen.findByText("No event selected")).toBeInTheDocument();
    expect(screen.queryByLabelText("Replay of recorded decisions")).toBeNull();
  });

  it("renders an explicit unavailable state rather than an empty success", async () => {
    settingsHandler();
    server.use(
      http.get("*/v1/work/causal", () =>
        HttpResponse.json({ code: "scope_denied" }, { status: 403 }),
      ),
    );
    trace();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Trace unavailable",
    );
    expect(screen.queryByText(/replayed identically/)).toBeNull();
  });

  it("renders a recorded event with nothing left to trace as a gap, not as nothing happening", async () => {
    settingsHandler();
    traceHandler({
      ...causalPageFixture,
      events: [],
      total: 0,
      returned: 0,
      gaps: ["decision:d"],
      gap_count: 1,
      complete: false,
    });
    trace();
    expect(
      await screen.findByText("Nothing recorded for this event"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/not proof that nothing happened/),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Trace completeness")).toHaveTextContent(
      "This trace is incomplete",
    );
  });

  it("renders one event and its resolved and unknown edges", async () => {
    settingsHandler();
    traceHandler(causalPageFixture);
    trace();
    expect(await screen.findByText("decision:d")).toBeInTheDocument();
    expect(screen.getByLabelText("Causes of decision:d")).toHaveTextContent(
      "caused by evidence:f",
    );
    expect(screen.getByLabelText("Causes of evidence:f")).toHaveTextContent(
      "the producer did not record what caused this",
    );
    expect(screen.getByText("Evidence")).toBeInTheDocument();
  });

  it("shows a cause the trace does not carry as a gap instead of an edge", async () => {
    settingsHandler();
    traceHandler({
      ...causalPageFixture,
      events: [causalPageFixture.events[0]],
      total: 1,
      returned: 1,
      gaps: ["evidence:f"],
      gap_count: 1,
      complete: false,
    });
    trace();
    expect(
      await screen.findByText("Gap: evidence:f is not in this trace"),
    ).toBeInTheDocument();
    expect(screen.queryByText("caused by evidence:f")).toBeNull();
    expect(screen.getByLabelText("Trace completeness")).toHaveTextContent(
      "1 cause could not be resolved",
    );
  });

  it("names an absent durable ledger as a reason the trace is incomplete", async () => {
    settingsHandler();
    traceHandler({
      ...causalPageFixture,
      durable: false,
      gaps: [],
      gap_count: 0,
      complete: false,
    });
    trace();
    expect(
      await screen.findByLabelText("Trace completeness"),
    ).toHaveTextContent("the durable ledger could not be read");
  });

  it("never claims a complete trace while gaps are reported", async () => {
    settingsHandler();
    traceHandler({ ...causalPageFixture, complete: true });
    trace();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Trace unavailable",
    );
  });

  it("pages a large trace instead of rendering every node", async () => {
    settingsHandler();
    const page: CausalPage = {
      ...causalPageFixture,
      root: "event:10000",
      events: Array.from({ length: 50 }, (_, index) => event(10_000 - index)),
      total: 10_000,
      returned: 50,
      truncated: true,
      next_cursor: "w1.next",
      gaps: [],
      gap_count: 0,
      complete: true,
      replay: { ...causalPageFixture.replay, steps: 10_000 },
    };
    traceHandler(page);
    trace("event:10000");
    expect(await screen.findByText("event:10000")).toBeInTheDocument();
    expect(screen.getAllByText(/^event:\d+$/)).toHaveLength(50);
    expect(screen.getByText(/10000 recorded steps/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next page" })).toBeEnabled();
  });

  it("reports a replay divergence with the diverging step named", async () => {
    settingsHandler();
    traceHandler({
      ...causalPageFixture,
      replay: {
        ...causalPageFixture.replay,
        steps: 2,
        finding_count: 1,
        findings: [
          {
            step: "spawn:child",
            scope: "child",
            divergence: {
              kind: "cause_not_yet_admitted",
              cause: "decision:d",
            },
          },
        ],
      },
    });
    trace();
    expect(
      await screen.findByLabelText("Replay divergences"),
    ).toHaveTextContent("spawn:child");
    expect(
      screen.getByText(/Cause decision:d had not been admitted/),
    ).toBeInTheDocument();
    expect(screen.getByText("1 divergence")).toBeInTheDocument();
  });

  it("states that replay does not reproduce model output and claims nothing more", async () => {
    settingsHandler();
    traceHandler(causalPageFixture);
    trace();
    const replay = await screen.findByLabelText("Replay of recorded decisions");
    expect(replay).toHaveTextContent("does not re-run any model");
    expect(replay).toHaveTextContent("does not reproduce model output");
    expect(replay).toHaveTextContent(
      "2 recorded coordination steps replayed identically.",
    );
    const copy = replay.textContent ?? "";
    for (const claim of [
      /\breproduces model output/,
      /\breproduces the model/,
      /same tokens\b/,
      /identical model output/,
      /replays the model\b/,
    ]) {
      expect(copy).not.toMatch(claim);
    }
    expect(copy).toMatch(/coordination decisions only/);
  });
});
