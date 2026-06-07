import { beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import type { CapsResponse } from "../../services/refact";
import type { ProviderDefaults } from "../../services/refact/providers";
import { server } from "../../utils/mockServer";
import { render, screen, within } from "../../utils/test-utils";
import { DefaultModels } from "./DefaultModels";

vi.mock("../../components/Chat/ModelSelector", () => ({
  ModelSelector: ({
    capability = "chat",
    value,
    onValueChange,
    defaultValue,
    allowUnset,
  }: {
    capability?: "chat" | "completion" | "embedding";
    value?: string;
    onValueChange: (model: string) => void;
    defaultValue?: string;
    allowUnset?: boolean;
  }) => {
    const options =
      capability === "completion"
        ? [
            "openai/qwen2.5/coder/0.5b/base",
            "openai/qwen2.5/coder/1.5b/base",
            "openai/qwen2.5/coder/3b/base",
          ]
        : capability === "embedding"
          ? ["openai/thenlper/gte-base"]
          : ["openai/gpt-4o", "openai/gpt-4o-mini"];
    const effectiveValue = value ?? defaultValue ?? "";
    const showUnavailable = Boolean(
      effectiveValue && !options.includes(effectiveValue),
    );

    return (
      <select
        aria-label={`${capability}-model`}
        value={effectiveValue}
        onChange={(event) => onValueChange(event.currentTarget.value)}
      >
        {allowUnset && <option value="">None</option>}
        {showUnavailable && (
          <option value={effectiveValue}>Unavailable: {effectiveValue}</option>
        )}
        {options.map((option) => (
          <option key={option} value={option}>
            {option}
          </option>
        ))}
      </select>
    );
  },
}));

vi.mock("../../components/ModelSamplingParams", () => ({
  ModelSamplingParams: ({
    capability = "chat",
  }: {
    capability?: "chat" | "completion" | "embedding";
  }) => (capability === "embedding" ? null : <div>Max tokens</div>),
}));

const config = {
  apiKey: "test",
  host: "web" as const,
  dev: true,
  themeProps: {},
  lspPort: 8001,
};

const baseDefaults: ProviderDefaults = {
  chat: { model: "openai/gpt-4o", max_new_tokens: 4096 },
  chat_model_2: {},
  task_planner_agent_model: {},
  chat_light: {},
  chat_thinking: {},
  chat_buddy: {},
  completion_model: "openai/qwen2.5/coder/1.5b/base",
  embedding_model: "openai/thenlper/gte-base",
  preserved_field: { nested: true },
};

function caps(): CapsResponse {
  return structuredClone(STUB_CAPS_RESPONSE);
}

function renderDefaultModels(args?: {
  defaults?: ProviderDefaults;
  caps?: CapsResponse;
  onSave?: (body: ProviderDefaults) => void;
}) {
  const onSave = args?.onSave ?? vi.fn();
  server.use(
    http.get("*/v1/ping", () => HttpResponse.text("pong")),
    http.get("*/v1/caps", () => HttpResponse.json(args?.caps ?? caps())),
    http.get("*/v1/defaults", () =>
      HttpResponse.json(args?.defaults ?? baseDefaults),
    ),
    http.post("*/v1/defaults", async ({ request }) => {
      onSave((await request.json()) as ProviderDefaults);
      return HttpResponse.json({ success: true });
    }),
  );

  return {
    onSave,
    ...render(
      <DefaultModels
        backFromDefaultModels={vi.fn()}
        host="web"
        tabbed={false}
      />,
      { preloadedState: { config } },
    ),
  };
}

async function selectByName(name: string) {
  return screen.findByRole("combobox", { name });
}

describe("DefaultModels", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  test("completion default selector renders completion models", async () => {
    renderDefaultModels();

    const selector = await selectByName("completion-model");

    expect(
      within(selector).getByRole("option", {
        name: "openai/qwen2.5/coder/0.5b/base",
      }),
    ).toBeInTheDocument();
    expect(
      within(selector).queryByRole("option", { name: "openai/gpt-4o-mini" }),
    ).not.toBeInTheDocument();
  });

  test("embedding default selector renders embedding model", async () => {
    renderDefaultModels();

    const selector = await selectByName("embedding-model");

    expect(
      within(selector).getByRole("option", {
        name: "openai/thenlper/gte-base",
      }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Max tokens")).toHaveLength(2);
  });

  test("save payload includes completion and embedding defaults while preserving unknown fields", async () => {
    const onSave = vi.fn();
    const { user } = renderDefaultModels({ onSave });

    await user.selectOptions(
      await selectByName("completion-model"),
      "openai/qwen2.5/coder/3b/base",
    );
    await user.selectOptions(
      await selectByName("embedding-model"),
      "openai/thenlper/gte-base",
    );

    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({
        completion_model: "openai/qwen2.5/coder/3b/base",
        embedding_model: "openai/thenlper/gte-base",
        preserved_field: { nested: true },
        chat: expect.objectContaining({ model: "openai/gpt-4o" }),
      }),
    );
  });

  test("clears unavailable saved defaults", async () => {
    const onSave = vi.fn();
    const { user } = renderDefaultModels({
      defaults: {
        ...baseDefaults,
        completion_model: "missing/completion",
        embedding_model: "missing/embedding",
      },
      onSave,
    });

    expect(
      within(await selectByName("completion-model")).getByRole("option", {
        name: "Unavailable: missing/completion",
      }),
    ).toBeInTheDocument();
    await user.selectOptions(await selectByName("completion-model"), "");

    expect(
      within(await selectByName("embedding-model")).getByRole("option", {
        name: "Unavailable: missing/embedding",
      }),
    ).toBeInTheDocument();
    await user.selectOptions(await selectByName("embedding-model"), "");

    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({
        completion_model: "",
        embedding_model: "",
      }),
    );
  });

  test("chat defaults still save as before", async () => {
    const onSave = vi.fn();
    const { user } = renderDefaultModels({ onSave });

    await user.selectOptions(
      (await screen.findAllByRole("combobox", { name: "chat-model" }))[0],
      "openai/gpt-4o-mini",
    );

    await user.click(screen.getByRole("button", { name: "Save Changes" }));

    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({
        chat: expect.objectContaining({
          model: "openai/gpt-4o-mini",
          max_new_tokens: 4096,
        }),
        completion_model: "openai/qwen2.5/coder/1.5b/base",
        embedding_model: "openai/thenlper/gte-base",
      }),
    );
  });
});
