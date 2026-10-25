import { describe, expect, it } from "vitest";
import { effectiveClaudeBackend, isFieldVisible } from "./schemaConditions";
import { parseSchema } from "./useProviderForm";
import { providerBaseOptions } from "../ProvidersView/providerInstanceUtils";

describe("Claude backend schema", () => {
  it("parses options and evaluates AND conditions with OR values and defaults", async () => {
    const { fields } = await parseSchema(`fields:
  backend:
    f_default: oauth
    f_options:
      - {value: oauth, label: Subscription}
      - {value: custom, label: Custom}
  auth_type:
    f_default: api_key
  api_key:
    f_secret: true
    f_visible_when:
      backend: [anthropic, custom]
      auth_type: [api_key]
`);
    expect(fields[0].f_options).toHaveLength(2);
    expect(isFieldVisible(fields[2], fields, {})).toBe(false);
    expect(isFieldVisible(fields[2], fields, { backend: "custom" })).toBe(true);
    expect(
      isFieldVisible(fields[2], fields, {
        backend: "custom",
        auth_type: "none",
      }),
    ).toBe(false);
    expect(isFieldVisible(fields[2], fields, { backend: "bedrock" })).toBe(
      false,
    );
  });

  it("defaults only recognized legacy bases without changing identity", () => {
    expect(effectiveClaudeBackend("claude_code", {})).toBe("oauth");
    expect(effectiveClaudeBackend("anthropic", {})).toBe("anthropic");
    expect(effectiveClaudeBackend("anthropic", { backend: "vertex" })).toBe(
      "anthropic",
    );
    expect(effectiveClaudeBackend("custom", {})).toBeUndefined();
    const legacy = {
      name: "anthropic_work",
      base_provider: "anthropic",
      display_name: "Work",
      enabled: true,
      readonly: false,
      has_credentials: true,
      status: "active" as const,
      model_count: 1,
    };
    expect(providerBaseOptions([legacy])).toEqual([
      { id: "claude_code", label: "Claude Code" },
    ]);
    expect(legacy.name).toBe("anthropic_work");
  });

  it("combines alternative condition records with common conditions", async () => {
    const { fields } = await parseSchema(`fields:
  auth_type:
    f_default: api_key
  api_key:
    f_visible_when: {enabled: ["true"]}
    f_visible_when_any:
      - {backend: [anthropic, foundry]}
      - {backend: [custom], auth_type: [api_key]}
`);
    const visible = (values: Record<string, unknown>) =>
      isFieldVisible(fields[1], fields, values);
    expect(
      visible({ enabled: true, backend: "anthropic", auth_type: "none" }),
    ).toBe(true);
    expect(visible({ enabled: true, backend: "custom" })).toBe(true);
    expect(
      visible({ enabled: true, backend: "custom", auth_type: "bearer" }),
    ).toBe(false);
    expect(visible({ enabled: false, backend: "foundry" })).toBe(false);
    expect(visible({ enabled: true, backend: "vertex" })).toBe(false);
  });
});
