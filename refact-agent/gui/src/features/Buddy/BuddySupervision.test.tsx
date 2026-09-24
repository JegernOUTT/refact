import { StrictMode } from "react";
import { act } from "@testing-library/react";
import { useBuddyEditorialDelivery } from "./useBuddyEditorialDelivery";
import { updateConfig } from "../Config/configSlice";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { fireEvent, render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { BuddySupervision } from "./BuddySupervision";
import type {
  MandateRequest,
  SupervisionContext,
} from "../../services/refact/buddySupervision";

const config = {
  config: { host: "vscode" as const, lspPort: 8001, themeProps: {} },
};
const context: SupervisionContext = {
  schema_version: 1,
  project: { project_id: "p", canonical_root: "/fixture" },
  worker: { worker_id: "w", engine_instance_id: "e", epoch: 1 },
  authenticated_owner: {
    source_kind: "external",
    source_id: "user",
    source_version: "http-v1",
  },
  features: {
    storage: true,
    supervision: true,
    editorial: false,
    actions: false,
    improvement: false,
  },
};
async function openGrantForm() {
  fireEvent.click(
    await screen.findByRole("button", { name: "Grant a bounded mandate" }),
  );
  await waitFor(() =>
    expect(screen.getByLabelText("Exact model ID")).toBeEnabled(),
  );
}
function fillGrant() {
  fireEvent.change(screen.getByLabelText("Exact model ID"), {
    target: { value: "provider/model" },
  });
  const expires = new Date(Date.now() + 3600000);
  const local = new Date(
    expires.getTime() - expires.getTimezoneOffset() * 60000,
  )
    .toISOString()
    .slice(0, 16);
  fireEvent.change(
    screen.getByLabelText("Mandate expires (local time, within 24 hours)"),
    { target: { value: local } },
  );
  fireEvent.click(
    screen.getByRole("checkbox", { name: /I grant only the exact project/ }),
  );
}
describe("Buddy supervision controls", () => {
  it("shows disabled rollout without mutating settings or granting authority", async () => {
    let writes = 0;
    server.use(
      http.post("*/v1/buddy/*", () => {
        writes++;
        return HttpResponse.json({});
      }),
    );
    let reads = 0;
    server.use(
      http.get("*/v1/buddy/supervision/context", () => {
        reads++;
        return HttpResponse.json({
          schema_version: 1,
          project: null,
          worker: null,
          authenticated_owner: null,
          features: {
            storage: false,
            supervision: false,
            editorial: false,
            actions: false,
            improvement: false,
          },
        });
      }),
    );
    render(<BuddySupervision />, { preloadedState: config });
    await waitFor(() => expect(reads).toBeGreaterThan(0));
    expect(screen.queryByLabelText("Buddy supervision")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Grant a bounded mandate" }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Grant bounded mandate" }),
    ).toBeNull();
    expect(writes).toBe(0);
  });

  it("surfaces a failed context load as an error with a retry, never as a silent panel", async () => {
    let reads = 0;
    let offline = true;
    server.use(
      http.get("*/v1/buddy/supervision/context", () => {
        reads++;
        return offline
          ? HttpResponse.json({}, { status: 503 })
          : HttpResponse.json(context);
      }),
    );
    render(<BuddySupervision />, { preloadedState: config });
    expect(
      await screen.findByText(/Supervision context could not be loaded/),
    ).toBeInTheDocument();
    const seen = reads;
    offline = false;
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(
      await screen.findByRole("button", { name: "Grant a bounded mandate" }),
    ).toBeInTheDocument();
    expect(reads).toBeGreaterThan(seen);
  });

  it("keeps the grant form collapsed until it is explicitly opened", async () => {
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json(context),
      ),
    );
    render(<BuddySupervision />, { preloadedState: config });
    await screen.findByRole("button", { name: "Grant a bounded mandate" });
    expect(screen.queryByLabelText("Exact model ID")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Grant bounded mandate" }),
    ).toBeNull();
    await openGrantForm();
    expect(screen.getByLabelText("Exact model ID")).toBeInTheDocument();
  });

  it("explains the missing owner before opening the disabled mandate form", async () => {
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json({ ...context, authenticated_owner: null }),
      ),
    );
    render(<BuddySupervision />, { preloadedState: config });

    const reason = await screen.findByText(
      "Granting a mandate needs this project's signed-in owner.",
    );
    const disclosure = screen.getByRole("button", {
      name: "Grant a bounded mandate",
    });

    expect(reason).toBeVisible();
    expect(disclosure).toHaveAccessibleDescription(reason.textContent);
    expect(disclosure).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByLabelText("Exact model ID")).toBeNull();
  });

  it("keeps the whole read-tool list out of the worker scope options", async () => {
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json(context),
      ),
    );
    render(<BuddySupervision />, { preloadedState: config });
    await openGrantForm();
    const scope = screen.getByLabelText("Worker scope");
    for (const option of Array.from(scope.querySelectorAll("option")))
      expect(option.textContent ?? "").not.toMatch(/,/);
    fireEvent.change(scope, { target: { value: "read_only" } });
    expect(screen.getByText(/^Read tools:/)).toBeInTheDocument();
  });

  it("marks the mandate fields as visibly disabled while storage is unavailable", async () => {
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json({
          ...context,
          features: { ...context.features, storage: false },
        }),
      ),
    );
    render(<BuddySupervision />, { preloadedState: config });
    fireEvent.click(
      await screen.findByRole("button", { name: "Grant a bounded mandate" }),
    );
    const group = await screen.findByRole("group", {
      name: "Explicit, expiring mandate",
    });
    expect(group).toHaveAttribute("data-disabled", "true");
    expect(screen.getByLabelText("Exact model ID")).toBeDisabled();
    expect(
      screen.getByText(/Buddy storage is unavailable/),
    ).toBeInTheDocument();
  });
  it("requires renewed confirmation after editing and never repeats uncertain grant", async () => {
    let writes = 0;
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json(context),
      ),
      http.post("*/v1/buddy/supervision/mandates", () => {
        writes++;
        return HttpResponse.json({}, { status: 503 });
      }),
      http.post("*/v1/buddy/supervision/mandates/:id/revoke", () =>
        HttpResponse.json({}, { status: 409 }),
      ),
    );
    render(<BuddySupervision />, { preloadedState: config });
    await openGrantForm();
    fillGrant();
    fireEvent.change(screen.getByLabelText("Maximum turns (1–100)"), {
      target: { value: "5" },
    });
    expect(
      screen.getByRole("checkbox", { name: /I grant only the exact project/ }),
    ).not.toBeChecked();
    expect(
      screen.getByRole("button", { name: "Grant bounded mandate" }),
    ).toBeDisabled();
    fireEvent.click(
      screen.getByRole("checkbox", { name: /I grant only the exact project/ }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Grant bounded mandate" }),
    );
    expect(
      await screen.findByText(/Request unconfirmed. Do not repeat intake/),
    ).toBeInTheDocument();
    expect(writes).toBe(1);
    expect(
      screen.queryByRole("button", { name: "Grant bounded mandate" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Stop and revoke mandate" }),
    ).toBeEnabled();
  });
  it("creates exact bounded grant then paused intake, never automatically spawns", async () => {
    let grant: MandateRequest | undefined;
    let intake: unknown;
    let spawns = 0;
    const link = {
      schema_version: 1,
      project: context.project,
      worker: context.worker,
      mandate_id: "",
      chat_id: "native-chat",
      command_acknowledged: true,
      dispatch_attempted: false,
      revoked: false,
      escalation: null,
      delivery_run_id: null,
    };
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json(context),
      ),
      http.post("*/v1/buddy/supervision/mandates", async ({ request }) => {
        grant = (await request.json()) as MandateRequest;
        link.mandate_id = grant.mandate.mandate_id;
        return HttpResponse.json({
          revision: 1,
          change_seq: 1,
          value: {
            schema_version: 1,
            project: context.project,
            worker: context.worker,
            mandate: grant.mandate,
          },
        });
      }),
      http.post(
        "*/v1/buddy/supervision/mandates/:id/intake",
        async ({ request }) => {
          intake = await request.json();
          return HttpResponse.json(link);
        },
      ),
      http.get("*/v1/buddy/supervision/mandates/:id/status", () =>
        HttpResponse.json({
          project: context.project,
          worker: context.worker,
          link,
          native_goal: { status: "paused" },
          native_worker: null,
          native_worker_goal: null,
          coverage: "Native observations only",
          host_wake_supported: false,
          daemon_wake_supported: false,
        }),
      ),
      http.post("*/v1/buddy/supervision/mandates/:id/spawn", () => {
        spawns++;
        return HttpResponse.json(link);
      }),
    );
    render(<BuddySupervision />, { preloadedState: config });
    await openGrantForm();
    fillGrant();
    fireEvent.click(
      screen.getByRole("button", { name: "Grant bounded mandate" }),
    );
    await screen.findByRole("button", { name: "Create paused native goal" });
    expect(grant?.mandate.model_constraints).toEqual(["provider/model"]);
    expect(grant?.max_workers).toBe(1);
    fireEvent.change(screen.getByLabelText("Objective"), {
      target: { value: "Explain the design" },
    });
    fireEvent.change(
      screen.getByLabelText("Explicit success criteria, one per line"),
      { target: { value: "Explain limitations" } },
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Create paused native goal" }),
    );
    await screen.findByText("Native observations only");
    expect(intake).toEqual({
      content: "Explain the design",
      criteria: [{ id: "C1", text: "Explain limitations", verify_hint: null }],
    });
    expect(spawns).toBe(0);
    expect(
      screen.getByText(/No verified delivery observed/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Start one bounded analysis worker" }),
    ).toBeEnabled();
  });
});

const speech = {
  id: "speech-1",
  text: "Evidence-backed editorial message",
  editorial_delivery_required: true,
};
function Delivery({
  surface = "panel",
  marked = true,
}: {
  surface?: "panel" | "avatar";
  marked?: boolean;
}) {
  const delivery = useBuddyEditorialDelivery(
    { ...speech, editorial_delivery_required: marked },
    surface,
  );
  return (
    <div>
      {delivery.speech?.text}
      {delivery.canFeedback && (
        <button onClick={() => void delivery.sendFeedback("useful")}>
          Helpful
        </button>
      )}
      {delivery.feedbackStatus}
    </div>
  );
}
describe("editorial HTTP integration", () => {
  it("claims once before text delivery and records explicit feedback with project and worker", async () => {
    const claims: unknown[] = [],
      feedback: unknown[] = [];
    let finish: () => void = () => {
      throw new Error("not started");
    };
    const gate = new Promise<void>((resolve) => {
      finish = resolve;
    });
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json({
          ...context,
          features: { ...context.features, editorial: true },
        }),
      ),
      http.post("*/v1/buddy/editorial/claim", async ({ request }) => {
        claims.push(await request.json());
        await gate;
        return HttpResponse.json({ claimed: true, permission_granted: false });
      }),
      http.post("*/v1/buddy/editorial/feedback", async ({ request }) => {
        feedback.push(await request.json());
        return HttpResponse.json({ recorded: true, permission_granted: false });
      }),
    );
    render(
      <StrictMode>
        <Delivery />
      </StrictMode>,
      { preloadedState: config },
    );
    await waitFor(() => expect(claims).toHaveLength(1));
    expect(screen.queryByText(speech.text)).toBeNull();
    finish();
    await screen.findByText(speech.text);
    expect(claims[0]).toEqual({
      project: context.project,
      worker: context.worker,
      speech_id: speech.id,
      surface: "panel",
    });
    expect(feedback).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Helpful" }));
    await screen.findByText(/Feedback recorded. Permissions unchanged./);
    expect(feedback[0]).toEqual({
      project: context.project,
      worker: context.worker,
      speech_id: speech.id,
      feedback: "useful",
    });
  });
  it.each([false, "offline", "permission"])(
    "withholds denied or invalid acknowledgement %s without retry",
    async (value) => {
      let claims = 0;
      server.use(
        http.get("*/v1/buddy/supervision/context", () =>
          HttpResponse.json({
            ...context,
            features: { ...context.features, editorial: true },
          }),
        ),
        http.post("*/v1/buddy/editorial/claim", () => {
          claims++;
          return value === "offline"
            ? HttpResponse.json({}, { status: 503 })
            : HttpResponse.json({
                claimed: value === "permission",
                permission_granted: value === "permission",
              });
        }),
      );
      const view = render(<Delivery />, { preloadedState: config });
      await waitFor(() => expect(claims).toBe(1));
      await act(async () => {
        await Promise.resolve();
      });
      view.rerender(<Delivery />);
      expect(screen.queryByText(speech.text)).toBeNull();
      expect(claims).toBe(1);
    },
  );
  it("retains unmarked legacy avatar text but never claims editorial avatar speech", () => {
    const view = render(<Delivery surface="avatar" />, {
      preloadedState: config,
    });
    expect(screen.queryByText(speech.text)).toBeNull();
    view.rerender(<Delivery surface="avatar" marked={false} />);
    expect(screen.getByText(speech.text)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Helpful" })).toBeNull();
  });
  it("withholds a late response after changing endpoint", async () => {
    let started = false;
    let finish: () => void = () => {
      throw new Error("not started");
    };
    const gate = new Promise<void>((resolve) => {
      finish = resolve;
    });
    server.use(
      http.get("http://127.0.0.1:8001/v1/buddy/supervision/context", () =>
        HttpResponse.json({
          ...context,
          features: { ...context.features, editorial: true },
        }),
      ),
      http.post("*/v1/buddy/editorial/claim", async () => {
        started = true;
        await gate;
        return HttpResponse.json({ claimed: true, permission_granted: false });
      }),
    );
    const view = render(<Delivery />, { preloadedState: config });
    await waitFor(() => expect(started).toBe(true));
    act(() => {
      view.store.dispatch(updateConfig({ lspPort: 8002 }));
    });
    await act(async () => {
      finish();
      await gate;
    });
    expect(screen.queryByText(speech.text)).toBeNull();
  });
});
