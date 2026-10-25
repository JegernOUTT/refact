import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { useProviderForm } from "./useProviderForm";

function Harness() {
  const form = useProviderForm({ providerName: "claude_code_work" });
  return (
    <>
      <div data-testid="important">
        {form.importantFields.map((field) => field.key).join(",")}
      </div>
      <div data-testid="advanced">
        {form.extraFields.map((field) => field.key).join(",")}
      </div>
      <div data-testid="secret">{String(form.formValues?.api_key ?? "")}</div>
      <button onClick={() => void form.handleFieldSave("backend", "bedrock")}>
        Switch backend
      </button>
    </>
  );
}

describe("backend partial saves", () => {
  it("filters before field splitting and switches backend without sending hidden secrets or changing legacy identity", async () => {
    const settings: Record<string, unknown> = {
      backend: "anthropic",
      api_key: "***",
      aws_profile: "work",
    };
    let payload: unknown;
    const provider = {
      name: "claude_code_work",
      base_provider: "claude_code",
      display_name: "Legacy Work",
      enabled: true,
      readonly: false,
      has_credentials: true,
      status: "active",
      model_count: 1,
    };
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [provider] }),
      ),
      http.get("*/v1/providers/claude_code_work", () =>
        HttpResponse.json({ ...provider, settings, runtime: null }),
      ),
      http.get("*/v1/providers/claude_code_work/schema", () =>
        HttpResponse.json({
          name: provider.name,
          schema: `fields:
  backend:
    f_default: oauth
  api_key:
    f_visible_when: {backend: [anthropic]}
  aws_profile:
    f_extra: true
    f_visible_when: {backend: [bedrock]}
`,
        }),
      ),
      http.post("*/v1/providers/claude_code_work", async ({ request }) => {
        payload = await request.json();
        Object.assign(settings, payload);
        return HttpResponse.json({ success: true });
      }),
    );
    const { user } = render(<Harness />, {
      preloadedState: {
        config: {
          apiKey: "test",
          lspPort: 8001,
          themeProps: {},
          host: "vscode",
        },
      },
    });
    await waitFor(() =>
      expect(screen.getByTestId("important")).toHaveTextContent(
        "backend,api_key",
      ),
    );
    expect(screen.getByTestId("advanced")).toBeEmptyDOMElement();
    await user.click(screen.getByRole("button", { name: "Switch backend" }));
    await waitFor(() =>
      expect(screen.getByTestId("advanced")).toHaveTextContent("aws_profile"),
    );
    expect(screen.getByTestId("important")).toHaveTextContent(/^backend$/);
    expect(payload).toEqual({
      base_provider: "claude_code",
      display_name: "Legacy Work",
      backend: "bedrock",
    });
    expect(settings.api_key).toBe("***");
    expect(screen.getByTestId("secret")).toHaveTextContent("***");
  });
});
