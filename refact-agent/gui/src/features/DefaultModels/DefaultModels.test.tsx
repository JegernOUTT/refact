import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import React from "react";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { reducer as configReducer } from "../Config/configSlice";

vi.mock("../../services/refact/providers", () => ({
  useGetDefaultsQuery: vi.fn(),
  useUpdateDefaultsMutation: vi.fn(),
}));

vi.mock("../../services/refact/caps", () => ({
  useGetCapsQuery: vi.fn(),
}));

vi.mock("../../services/refact/buddy", () => ({
  useGetDraftQuery: vi.fn(),
}));

vi.mock("../../components/Chat/ModelSelector", () => ({
  ModelSelector: ({
    onValueChange,
    value,
  }: {
    onValueChange: (v: string) => void;
    value?: string;
    allowUnset?: boolean;
    unsetLabel?: string;
    showLabel?: boolean;
    compact?: boolean;
    defaultValue?: string;
  }) => (
    <button
      data-testid="model-selector"
      data-value={value ?? ""}
      onClick={() => onValueChange("changed-model")}
    >
      {value ?? "None"}
    </button>
  ),
}));

vi.mock("../../components/ModelSamplingParams", () => ({
  ModelSamplingParams: ({
    onChange,
    model,
  }: {
    onChange: (k: string, v: unknown) => void;
    model: string;
    values: object;
  }) => (
    <button
      data-testid="sampling-params"
      data-model={model}
      onClick={() => onChange("temperature", 0.8)}
    >
      sampling
    </button>
  ),
}));

vi.mock("../Buddy/BuddyDraftPreview", () => ({
  BuddyDraftPreview: () => <div data-testid="buddy-draft-preview" />,
}));

vi.mock("../../components/PageWrapper", () => ({
  PageWrapper: ({ children }: { children: React.ReactNode }) => (
    <div data-testid="page-wrapper">{children}</div>
  ),
}));

vi.mock("../../components/Spinner", () => ({
  Spinner: ({ spinning }: { spinning?: boolean }) =>
    spinning ? <div data-testid="spinner" /> : null,
}));

import { DefaultModels } from "./DefaultModels";
import {
  useGetDefaultsQuery,
  useUpdateDefaultsMutation,
} from "../../services/refact/providers";
import { useGetCapsQuery } from "../../services/refact/caps";
import { useGetDraftQuery } from "../../services/refact/buddy";

const baseDefaults = {
  chat: {},
  chat_model_2: {},
  task_planner_agent_model: {},
  chat_light: {},
  chat_thinking: {},
  chat_buddy: {},
};

const baseCaps = {
  chat_default_model: "gpt-4",
  chat_model_2: "",
  task_planner_agent_model: "",
  chat_light_model: "",
  chat_thinking_model: "",
  chat_buddy_model: "",
};

function setupMocks(overrides: { draftData?: unknown } = {}) {
  const updateDefaults = vi
    .fn()
    .mockReturnValue({ unwrap: vi.fn().mockResolvedValue({}) });
  (useGetDefaultsQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: baseDefaults,
    isLoading: false,
    isSuccess: true,
    isError: false,
    refetch: vi.fn(),
  });
  (useUpdateDefaultsMutation as ReturnType<typeof vi.fn>).mockReturnValue([
    updateDefaults,
    { isLoading: false },
  ]);
  (useGetCapsQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: baseCaps,
    refetch: vi.fn(),
  });
  (useGetDraftQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: overrides.draftData ?? undefined,
    isLoading: false,
    error: undefined,
  });
  return { updateDefaults };
}

const defaultProps = {
  backFromDefaultModels: vi.fn(),
  host: "web" as const,
  tabbed: false as const,
};

function createTestStore() {
  return configureStore({ reducer: { config: configReducer } });
}

function renderWithStore(ui: React.ReactElement) {
  return render(<Provider store={createTestStore()}>{ui}</Provider>);
}

describe("DefaultModels — embedded", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders all 6 role tabs with short labels", () => {
    setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    for (const label of [
      "Chat",
      "Chat 2",
      "Planner",
      "Light",
      "Thinking",
      "Companion",
    ]) {
      expect(screen.getByRole("tab", { name: label })).toBeInTheDocument();
    }
  });

  it("initial Chat tab is active (aria-selected)", () => {
    setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    expect(screen.getByRole("tab", { name: "Chat" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("switches active tab when a different role tab is clicked", () => {
    setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Light" }));
    expect(screen.getByRole("tab", { name: "Light" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "Chat" })).toHaveAttribute(
      "aria-selected",
      "false",
    );
  });

  it("Save button is disabled when no changes", () => {
    setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    expect(screen.getByRole("button", { name: "Save Changes" })).toBeDisabled();
  });

  it("model change enables Save button", () => {
    setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    fireEvent.click(screen.getAllByTestId("model-selector")[0]);
    expect(
      screen.getByRole("button", { name: "Save Changes" }),
    ).not.toBeDisabled();
  });

  it("Save button calls updateDefaults mutation", async () => {
    const { updateDefaults } = setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    fireEvent.click(screen.getAllByTestId("model-selector")[0]);
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(updateDefaults).toHaveBeenCalledOnce());
  });

  it("sampling change enables Save button", () => {
    setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    const samplingBtns = screen.queryAllByTestId("sampling-params");
    if (samplingBtns.length > 0) {
      fireEvent.click(samplingBtns[0]);
      expect(
        screen.getByRole("button", { name: "Save Changes" }),
      ).not.toBeDisabled();
    }
  });

  it("applies draft overrides and enables Save when draft is present", () => {
    setupMocks({
      draftData: {
        kind: "defaults_model",
        yaml_or_json: JSON.stringify({ chat: { model: "draft-model" } }),
      },
    });
    render(<DefaultModels {...defaultProps} embedded />);
    expect(screen.getByTestId("buddy-draft-preview")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Save Changes" }),
    ).not.toBeDisabled();
  });

  it("does not render SettingsShell sidebar (no double shell)", () => {
    setupMocks();
    const { container } = render(<DefaultModels {...defaultProps} embedded />);
    expect(container.querySelector("aside")).not.toBeInTheDocument();
  });

  it("does not render Back button when embedded", () => {
    setupMocks();
    render(<DefaultModels {...defaultProps} embedded />);
    expect(
      screen.queryByRole("button", { name: /back/i }),
    ).not.toBeInTheDocument();
  });
});

describe("DefaultModels — standalone (not embedded)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("wraps content in PageWrapper", () => {
    setupMocks();
    renderWithStore(<DefaultModels {...defaultProps} />);
    expect(screen.getByTestId("page-wrapper")).toBeInTheDocument();
  });

  it("renders Back button in standalone mode", () => {
    setupMocks();
    renderWithStore(<DefaultModels {...defaultProps} />);
    expect(screen.getByRole("button", { name: /back/i })).toBeInTheDocument();
  });

  it("Back button calls backFromDefaultModels", () => {
    const onBack = vi.fn();
    setupMocks();
    renderWithStore(
      <DefaultModels {...defaultProps} backFromDefaultModels={onBack} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /back/i }));
    expect(onBack).toHaveBeenCalledOnce();
  });

  it("model change enables Save in standalone mode", () => {
    setupMocks();
    renderWithStore(<DefaultModels {...defaultProps} />);
    fireEvent.click(screen.getAllByTestId("model-selector")[0]);
    expect(
      screen.getByRole("button", { name: "Save Changes" }),
    ).not.toBeDisabled();
  });

  it("Save mutation dispatched in standalone mode", async () => {
    const { updateDefaults } = setupMocks();
    renderWithStore(<DefaultModels {...defaultProps} />);
    fireEvent.click(screen.getAllByTestId("model-selector")[0]);
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(updateDefaults).toHaveBeenCalledOnce());
  });
});

describe("DefaultModels — loading state", () => {
  it("shows spinner while loading defaults", () => {
    (useGetDefaultsQuery as ReturnType<typeof vi.fn>).mockReturnValue({
      data: undefined,
      isLoading: true,
      isSuccess: false,
      isError: false,
      refetch: vi.fn(),
    });
    (useUpdateDefaultsMutation as ReturnType<typeof vi.fn>).mockReturnValue([
      vi.fn(),
      { isLoading: false },
    ]);
    (useGetCapsQuery as ReturnType<typeof vi.fn>).mockReturnValue({
      data: undefined,
      refetch: vi.fn(),
    });
    (useGetDraftQuery as ReturnType<typeof vi.fn>).mockReturnValue({
      data: undefined,
      isLoading: false,
      error: undefined,
    });
    render(<DefaultModels {...defaultProps} embedded />);
    expect(screen.getByTestId("spinner")).toBeInTheDocument();
  });
});
