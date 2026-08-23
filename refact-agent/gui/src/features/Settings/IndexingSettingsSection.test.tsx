import { screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  useGetIndexingSettingsQuery,
  useSaveIndexingSettingsMutation,
} from "../../services/refact/indexingSettings";
import type {
  IndexingSettingsResponse,
  IndexingSettingsScope,
} from "../../services/refact/indexingSettings";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import { render } from "../../utils/test-utils";
import { IndexingSettingsSection } from "./IndexingSettingsSection";

vi.mock("../../services/refact/indexingSettings", () => ({
  useGetIndexingSettingsQuery: vi.fn(),
  useSaveIndexingSettingsMutation: vi.fn(),
}));

vi.mock("../../services/refact/ragStatus", () => ({
  useGetRagStatusQuery: vi.fn(),
}));

const globalSettings: IndexingSettingsResponse = {
  scope: "global",
  path: "/home/user/.config/refact/indexing.yaml",
  project_available: true,
  config: {
    blocklist: ["*/generated/*", "*.min.js"],
    additional_indexing_dirs: ["/shared/library"],
  },
};

const projectSettings: IndexingSettingsResponse = {
  scope: "project",
  path: "/workspace/.refact/indexing.yaml",
  project_available: true,
  config: {
    blocklist: ["project-only/**"],
    additional_indexing_dirs: ["/workspace/vendor"],
  },
};

const saveSettings = vi.fn();

function mockSettingsByScope(
  settings: Partial<Record<IndexingSettingsScope, IndexingSettingsResponse>>,
) {
  (useGetIndexingSettingsQuery as ReturnType<typeof vi.fn>).mockImplementation(
    (scope: IndexingSettingsScope) => ({
      data: settings[scope],
      error: undefined,
      isFetching: false,
    }),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mockSettingsByScope({
    global: globalSettings,
    project: projectSettings,
  });
  saveSettings.mockReturnValue({
    unwrap: () => Promise.resolve(globalSettings),
  });
  (useSaveIndexingSettingsMutation as ReturnType<typeof vi.fn>).mockReturnValue(
    [saveSettings, { isLoading: false }],
  );
  (useGetRagStatusQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: undefined,
    error: undefined,
    isLoading: false,
  });
});

describe("IndexingSettingsSection", () => {
  it("renders the global configuration rows", async () => {
    render(<IndexingSettingsSection />);

    expect(screen.getByText("Blocklist globs")).toBeInTheDocument();
    expect(
      screen.getByText("Additional indexing directories"),
    ).toBeInTheDocument();
    expect(
      await screen.findByDisplayValue("*/generated/*"),
    ).toBeInTheDocument();
    expect(screen.getByDisplayValue("*.min.js")).toBeInTheDocument();
    expect(screen.getByDisplayValue("/shared/library")).toBeInTheDocument();
    expect(
      screen.getByText("/home/user/.config/refact/indexing.yaml"),
    ).toBeInTheDocument();
  });

  it("disables project scope when no project is available", async () => {
    mockSettingsByScope({
      global: { ...globalSettings, project_available: false },
    });

    render(<IndexingSettingsSection />);

    expect(
      await screen.findByRole("radio", { name: "This project" }),
    ).toBeDisabled();
    expect(
      screen.getByText(/No project is currently available/),
    ).toBeInTheDocument();
  });

  it("reads project configuration values after switching scope", async () => {
    const { user } = render(<IndexingSettingsSection />);

    await user.click(screen.getByRole("radio", { name: "This project" }));

    expect(
      await screen.findByDisplayValue("project-only/**"),
    ).toBeInTheDocument();
    expect(screen.getByDisplayValue("/workspace/vendor")).toBeInTheDocument();
    expect(screen.queryByDisplayValue("*/generated/*")).not.toBeInTheDocument();
    expect(useGetIndexingSettingsQuery).toHaveBeenLastCalledWith("project");
  });

  it("omits blank list values and saves the scope with its config", async () => {
    const { user } = render(<IndexingSettingsSection />);

    await user.click(screen.getByRole("radio", { name: "This project" }));
    await screen.findByDisplayValue("project-only/**");
    await user.click(screen.getByRole("button", { name: "Add glob" }));
    await user.click(screen.getByRole("button", { name: "Add directory" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(saveSettings).toHaveBeenCalledWith({
      scope: "project",
      config: {
        blocklist: ["project-only/**"],
        additional_indexing_dirs: ["/workspace/vendor"],
      },
    });
    expect(
      await screen.findByText("Indexing settings saved."),
    ).toBeInTheDocument();
  });

  it("displays error feedback when saving is rejected", async () => {
    saveSettings.mockReturnValue({
      unwrap: () => Promise.reject({ data: { detail: "Permission denied" } }),
    });
    const { user } = render(<IndexingSettingsSection />);

    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(screen.getByRole("alert")).toHaveTextContent(
        "Could not save indexing settings: Permission denied",
      );
    });
  });
});
