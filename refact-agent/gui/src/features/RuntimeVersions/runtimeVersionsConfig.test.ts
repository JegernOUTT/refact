import { describe, expect, it } from "vitest";
import {
  reducer,
  updateConfig,
  type ConfigUpdate,
} from "../Config/configSlice";

const observed = { state: "observed", version: "A" } as const;
const start = () =>
  reducer(
    undefined,
    updateConfig({
      host: "vscode",
      lspUrl: "http://localhost:8001",
      lspPort: 8001,
      daemonPort: 8000,
      runtimeVersions: {
        plugin: observed,
        selected: observed,
        installed: observed,
        staged: observed,
        daemon: observed,
        worker: observed,
        requestedVersion: "B",
      },
    }),
  );

describe("runtime version config snapshots", () => {
  it("supports legacy omission, atomic replacement and explicit clearing", () => {
    expect(
      reducer(undefined, updateConfig({})).runtimeVersions,
    ).toBeUndefined();
    expect(
      reducer(start(), updateConfig({ tabbed: true })).runtimeVersions,
    ).toEqual(start().runtimeVersions);
    expect(
      reducer(start(), updateConfig({ runtimeVersions: { plugin: observed } }))
        .runtimeVersions,
    ).toEqual({ plugin: { ...observed, commit: null } });
    expect(
      reducer(start(), updateConfig({ runtimeVersions: null })).runtimeVersions,
    ).toBeNull();
  });

  it.each([
    { lspUrl: "http://localhost:9001" },
    { lspUrl: null },
    { lspPort: 9001 },
    { daemonPort: 9000 },
    { daemonPort: undefined },
    { host: "web" },
    { dev: true },
    { engineServed: true },
  ] satisfies ConfigUpdate[])(
    "clears daemon and worker on identity change %j",
    (change) => {
      const result = reducer(start(), updateConfig(change)).runtimeVersions;
      expect(result?.daemon).toBeNull();
      expect(result?.worker).toBeNull();
      expect(result?.selected?.version).toBe("A");
      expect(result?.requestedVersion).toBe("B");
    },
  );

  it("retains a replacement snapshot supplied with the new identity", () => {
    expect(
      reducer(
        start(),
        updateConfig({
          daemonPort: 9000,
          runtimeVersions: { daemon: { ...observed, version: "new endpoint" } },
        }),
      ).runtimeVersions?.daemon?.version,
    ).toBe("new endpoint");
  });

  it("rejects invalid and unbounded fields at the incoming boundary", () => {
    const runtimeVersions = {
      requestedVersion: "x".repeat(161),
      plugin: { state: "invented", version: "wrong" },
      selected: {
        state: "observed",
        version: {},
        commit: "bad\ncommit",
        source: "private path",
        observedAtMs: Infinity,
      },
      worker: { state: "last_observed", version: " good ", observedAtMs: -1 },
      installed: ["bad"],
    };
    const result = reducer(
      start(),
      updateConfig({ runtimeVersions } as unknown as ConfigUpdate),
    ).runtimeVersions;
    expect(result).toEqual({
      requestedVersion: null,
      plugin: null,
      selected: { state: "observed", version: null, commit: null },
      worker: { state: "last_observed", version: "good", commit: null },
      installed: null,
    });
  });
});
