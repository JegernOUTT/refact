import { configureStore } from "@reduxjs/toolkit";
import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { render } from "../../utils/test-utils";
import { reducer as configReducer } from "../Config/configSlice";
import { pagesSlice } from "../Pages/pagesSlice";
import {
  useGetProjectInformationQuery,
  useSaveProjectInformationMutation,
} from "../../services/refact/projectInformation";
import { useGetRagStatusQuery } from "../../services/refact/ragStatus";
import {
  useGetSkillsSettingsQuery,
  useSaveSkillsSettingsMutation,
} from "../../services/refact/skillsSettings";
import { GeneralSettingsSection } from "./GeneralSettingsSection";

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.releasePointerCapture = () => undefined;

vi.mock("../../services/refact/projectInformation", () => ({
  useGetProjectInformationQuery: vi.fn(),
  useSaveProjectInformationMutation: vi.fn(),
}));

vi.mock("../../services/refact/ragStatus", () => ({
  useGetRagStatusQuery: vi.fn(),
}));

vi.mock("../../services/refact/skillsSettings", () => ({
  useGetSkillsSettingsQuery: vi.fn(),
  useSaveSkillsSettingsMutation: vi.fn(),
}));

vi.mock("../../hooks/useEventBusForIDE", () => ({
  useEventsBusForIDE: () => ({
    openHotKeys: vi.fn(),
    openSettings: vi.fn(),
  }),
}));

const projectConfig = {
  schema_version: 1,
  enabled: true,
  defaults: { max_chars_per_item: 8000, max_items_per_section: 50 },
  sections: {
    system_info: { enabled: true },
    environment_instructions: { enabled: true },
    detected_environments: { enabled: true },
    git_info: { enabled: true },
    project_tree: { enabled: true },
    instruction_files: { enabled: true },
    project_configs: { enabled: true },
    memories: { enabled: true },
  },
};

const saveProject = vi.fn(() => ({ unwrap: () => Promise.resolve(undefined) }));
const saveSkills = vi.fn(() => ({ unwrap: () => Promise.resolve(undefined) }));

function renderSection() {
  const store = configureStore({
    reducer: {
      config: configReducer,
      pages: pagesSlice.reducer,
    },
  });
  render(
    <Provider store={store}>
      <GeneralSettingsSection />
    </Provider>,
  );
  return store;
}

beforeEach(() => {
  vi.clearAllMocks();
  (useGetProjectInformationQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: projectConfig,
    isLoading: false,
    isError: false,
  });
  (
    useSaveProjectInformationMutation as ReturnType<typeof vi.fn>
  ).mockReturnValue([saveProject, { isLoading: false }]);
  (useGetSkillsSettingsQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: { auto_trigger: "index_only" },
    isLoading: false,
    isError: false,
  });
  (useSaveSkillsSettingsMutation as ReturnType<typeof vi.fn>).mockReturnValue([
    saveSkills,
    { isLoading: false },
  ]);
  (useGetRagStatusQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: {
      codegraph_alive: "working",
      codegraph: { state: "working" },
      vecdb_alive: "true",
      vecdb: { state: "done" },
      vec_db_error: "",
    },
    isError: false,
  });
});

describe("GeneralSettingsSection", () => {
  it("does not render legacy feature switches", () => {
    renderSection();

    expect(screen.queryByText("Feature Flags")).not.toBeInTheDocument();
    expect(screen.getAllByRole("switch")).toHaveLength(1);
    expect(
      screen.getByRole("switch", { name: "Inject project information" }),
    ).toBeInTheDocument();
    for (const label of ["Statistics", "Images", "AST", "VecDB"]) {
      expect(screen.queryByText(label)).not.toBeInTheDocument();
    }
  });

  it("persists only the changed project context master switch", async () => {
    const user = userEvent.setup();
    renderSection();

    await user.click(
      screen.getByRole("switch", { name: "Inject project information" }),
    );

    expect(saveProject).toHaveBeenCalledWith({
      ...projectConfig,
      enabled: false,
    });
  });

  it("saves the selected project skills behavior", async () => {
    const user = userEvent.setup();
    renderSection();

    await user.click(
      screen.getByRole("combobox", { name: "Automatic skill context" }),
    );
    await user.click(screen.getByRole("option", { name: "Inject full" }));

    expect(saveSkills).toHaveBeenCalledWith({ auto_trigger: "inject_full" });
  });

  it("navigates to indexing settings", async () => {
    const user = userEvent.setup();
    const store = renderSection();

    await user.click(
      screen.getByRole("button", { name: "Configure indexing" }),
    );

    expect(store.getState().pages.at(-1)).toEqual({
      name: "indexing settings",
    });
  });
});
