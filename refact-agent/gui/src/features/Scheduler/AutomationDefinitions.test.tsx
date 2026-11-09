import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { screen, render, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { AutomationDefinitions } from "./AutomationDefinitions";
import { isAutomationCatalog, parseDefinitionDraft } from "./automationTypes";

const definition = {
  schema_version: 1,
  id: "sensor",
  revision: 2,
  owner: { kind: "project" },
  description: "Sensor collection",
  enabled: true,
  trigger: { kind: "continuous" },
  action: {
    kind: "collect",
    driver: { type: "http_poll", url: "https://example.com/data" },
  },
  deliveries: [],
  lifecycle: { kind: "on_demand" },
  policy: {
    idempotent: true,
    max_pending_events: 1024,
    max_event_bytes: 262144,
  },
};
const catalog = {
  catalog_revision: 9,
  definitions: [
    {
      definition,
      status: {
        authorization: "needs_approval",
        execution: null,
        freshness: null,
        delivery: null,
      },
      capabilities: { desired_control: ["disable"] },
    },
  ],
};
describe("Automation boundary and controls", () => {
  it("rejects malformed wire dimensions and unsafe revisions", () => {
    expect(isAutomationCatalog(catalog)).toBe(true);
    expect(
      isAutomationCatalog({
        ...catalog,
        catalog_revision: Number.MAX_SAFE_INTEGER + 1,
      }),
    ).toBe(false);
    expect(
      isAutomationCatalog({
        ...catalog,
        definitions: [
          { ...catalog.definitions[0], status: { authorization: true } },
        ],
      }),
    ).toBe(false);
  });
  it("rejects inline secrets before dispatch while accepting credential references", () => {
    expect(() =>
      parseDefinitionDraft(
        JSON.stringify({
          ...definition,
          action: {
            kind: "collect",
            driver: { type: "http_poll", token: "private" },
          },
        }),
      ),
    ).toThrow("Inline credentials");
    const binding = {
      reference: "sensor-key",
      header: "authorization",
      scheme: "Bearer",
      expected_identity_revision: "identity-1",
    };
    const referenced = {
      ...definition,
      action: {
        ...definition.action,
        policy: {
          credential_refs: ["sensor-key"],
          credential_bindings: [binding],
        },
      },
    };
    expect(parseDefinitionDraft(JSON.stringify(referenced))).toEqual(
      referenced,
    );
    for (const invalid of [
      { ...binding, token: "private" },
      { ...binding, password: "private" },
      { ...binding, secret: "private" },
      { ...binding, value: "private" },
      { ...binding, header: { authorization: "private" } },
      { ...binding, scheme: "Bearer private" },
      {
        ...binding,
        expected_identity_revision: "https://example.com/?key=private",
      },
    ]) {
      expect(() =>
        parseDefinitionDraft(
          JSON.stringify({
            ...referenced,
            action: {
              ...referenced.action,
              policy: { credential_bindings: [invalid] },
            },
          }),
        ),
      ).toThrow("Inline credentials");
    }
    for (const policy of [
      { headers: { authorization: "private" } },
      { credential_bindings: [binding], headers: { "x-custom": "private" } },
    ]) {
      expect(() =>
        parseDefinitionDraft(
          JSON.stringify({
            ...definition,
            action: { ...definition.action, policy },
          }),
        ),
      ).toThrow("Inline credentials");
    }
    expect(() =>
      parseDefinitionDraft(
        JSON.stringify({
          ...referenced,
          action: {
            ...referenced.action,
            policy: { owner_definition_digest: "private" },
          },
        }),
      ),
    ).toThrow("host-only");
    expect(() =>
      parseDefinitionDraft(
        JSON.stringify({
          ...definition,
          action: {
            ...definition.action,
            driver: { ...definition.action.driver, endpoint: "private" },
          },
        }),
      ),
    ).toThrow("Unsupported collector driver fields");
    for (const url of [
      "https://example.com/?access_token=private",
      "https://example.com/?signature=private",
      "https://user:pass@example.com/",
      "wss://example.com/events#private",
      "https://example.com/?page=2",
    ]) {
      for (const extra of [
        { endpoint: url },
        { nested: [{ endpoint: url }] },
      ]) {
        expect(() =>
          parseDefinitionDraft(
            JSON.stringify({
              ...definition,
              action: {
                ...definition.action,
                policy: { websocket_subscribe: extra },
              },
            }),
          ),
        ).toThrow("Inline credentials");
      }
      expect(() =>
        parseDefinitionDraft(
          JSON.stringify({
            ...definition,
            action: { kind: "collect", driver: { type: "http_poll", url } },
          }),
        ),
      ).toThrow("Inline credentials");
    }
    expect(() => parseDefinitionDraft(" ".repeat(65537))).toThrow("64 KiB");
  });
  it("separates desired enablement from approval and sends exact CAS controls", async () => {
    const bodies: unknown[] = [];
    server.use(
      http.get("*/v1/scheduler/automation", () => HttpResponse.json(catalog)),
      http.post(
        "*/v1/scheduler/automation/:id/control",
        async ({ request }) => {
          bodies.push(await request.json());
          return HttpResponse.json({ catalog_revision: 10 });
        },
      ),
    );
    const { user } = render(<AutomationDefinitions view="continuous" />);
    expect(await screen.findByText("Sensor collection")).toBeInTheDocument();
    expect(screen.getByText("Enabled")).toBeInTheDocument();
    expect(screen.getAllByText("Not observed")).toHaveLength(3);
    expect(
      screen.queryByRole("button", { name: "Approve" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Run now" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Disable" }));
    await waitFor(() =>
      expect(bodies).toEqual([
        {
          expected_catalog_revision: 9,
          expected_revision: 2,
          operation: "disable",
        },
      ]),
    );
  });
  it("retains editor draft and original revisions on conflict without retrying", async () => {
    const bodies: unknown[] = [];
    server.use(
      http.get("*/v1/scheduler/automation", () => HttpResponse.json(catalog)),
      http.put("*/v1/scheduler/automation/:id", async ({ request }) => {
        bodies.push(await request.json());
        return HttpResponse.json(
          { detail: "catalog revision conflict" },
          { status: 409 },
        );
      }),
    );
    const { user } = render(<AutomationDefinitions view="continuous" />);
    await user.click(
      await screen.findByRole("button", { name: "Edit definition" }),
    );
    const draft = screen.getByRole("textbox", { name: "Definition JSON" });
    await user.click(screen.getByRole("button", { name: "Save definition" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "catalog revision conflict",
    );
    expect(draft).toHaveValue(
      JSON.stringify({ ...definition, revision: 3 }, null, 2),
    );
    expect(bodies).toEqual([
      {
        expected_catalog_revision: 9,
        expected_revision: 2,
        definition: { ...definition, revision: 3 },
      },
    ]);
  });
  it("shows actual source health and data age without attributing aggregate backlog to a definition", async () => {
    server.use(
      http.get("*/v1/scheduler/automation", () =>
        HttpResponse.json({
          ...catalog,
          definitions: [
            {
              ...catalog.definitions[0],
              status: {
                authorization: "approved",
                execution: {
                  state: "backoff",
                  generation: 1,
                  sequence: 2,
                  background: true,
                },
                freshness: {
                  data_age_ms: 42000,
                  last_data_at_ms: 1700000000000,
                  error_class: "network",
                },
                delivery: null,
              },
              project_wake: {
                next_wake_ms: null,
                background_active: true,
                pending_delivery: 7,
                recovery: 1,
              },
            },
          ],
        }),
      ),
    );
    render(<AutomationDefinitions view="continuous" />);
    expect(await screen.findByText("Backoff")).toBeInTheDocument();
    expect(screen.getByText(/42000 ms/)).toHaveTextContent("Network");
    expect(screen.getByText(/Project aggregate/)).toHaveTextContent(
      "7 pending deliveries",
    );
  });
  it("creates a file collector starter with bounded declarative fields and no secret entry", async () => {
    server.use(
      http.get("*/v1/scheduler/automation", () => HttpResponse.json(catalog)),
    );
    const { user } = render(<AutomationDefinitions view="continuous" />);
    await user.click(
      await screen.findByRole("button", { name: "New definition" }),
    );
    await user.type(
      screen.getByRole("textbox", { name: "Source URL or file path" }),
      "logs/events.jsonl",
    );
    await user.click(screen.getByRole("button", { name: "Use File tail" }));
    expect(
      screen.getByRole("textbox", { name: "Definition JSON" }),
    ).toHaveTextContent('"type": "file_tail"');
    expect(
      screen.queryByLabelText(/password|secret|token/i),
    ).not.toBeInTheDocument();
  });
});
