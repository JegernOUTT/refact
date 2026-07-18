import { describe, expect, it } from "vitest";
import { setUpStore } from "../app/store";
import {
  selectCapabilities,
  selectSurface,
  updateConfig,
  type Capabilities,
  type Config,
  type Surface,
} from "../features/Config/configSlice";
import { resolveCapabilities } from "../utils/capabilities";

const WEB_CAPABILITIES: Capabilities = {
  filesPanel: true,
  gitPanel: true,
  terminalPanel: true,
  openFileInApp: true,
  openFileInIde: false,
  ideDiffPasteBack: false,
  folderPicker: true,
};

const IDE_CAPABILITIES: Capabilities = {
  filesPanel: false,
  gitPanel: false,
  terminalPanel: false,
  openFileInApp: false,
  openFileInIde: true,
  ideDiffPasteBack: true,
  folderPicker: false,
};

describe("resolveCapabilities", () => {
  const matrix: readonly (readonly [Config["host"], Capabilities])[] = [
    ["web", WEB_CAPABILITIES],
    ["ide", IDE_CAPABILITIES],
    ["vscode", IDE_CAPABILITIES],
    ["jetbrains", IDE_CAPABILITIES],
  ];

  it.each(matrix)("resolves the %s host defaults", (host, expected) => {
    expect(resolveCapabilities(host)).toEqual(expected);
  });

  it("applies overrides after host defaults", () => {
    expect(
      resolveCapabilities("web", {
        terminalPanel: false,
        openFileInIde: true,
      }),
    ).toEqual({
      ...WEB_CAPABILITIES,
      terminalPanel: false,
      openFileInIde: true,
    });
  });
});

describe("config capability selectors", () => {
  it("defaults to the workspace surface and derives capabilities from host", () => {
    const store = setUpStore();

    expect(selectSurface(store.getState())).toBe("workspace");
    expect(selectCapabilities(store.getState())).toEqual(WEB_CAPABILITIES);

    store.dispatch(updateConfig({ host: "vscode" }));

    expect(selectCapabilities(store.getState())).toEqual(IDE_CAPABILITIES);
  });

  it("stores surface and capability overrides", () => {
    const store = setUpStore();

    store.dispatch(
      updateConfig({
        surface: "dashboard",
        capabilities: { terminalPanel: false },
      }),
    );

    expect(store.getState().config.surface).toBe("dashboard");
    expect(store.getState().config.capabilities).toEqual({
      terminalPanel: false,
    });
    expect(selectSurface(store.getState())).toBe("dashboard");
    expect(selectCapabilities(store.getState())).toEqual({
      ...WEB_CAPABILITIES,
      terminalPanel: false,
    });
  });

  it("falls back to workspace for an unknown runtime surface", () => {
    const store = setUpStore();

    store.dispatch(updateConfig({ surface: "unknown" as Surface }));

    expect(selectSurface(store.getState())).toBe("workspace");
  });
});
