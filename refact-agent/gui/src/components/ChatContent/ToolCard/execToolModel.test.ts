import { describe, expect, test } from "vitest";

import {
  execMetadataFromResult,
  isProcessStartCall,
  parseProcessSnapshotHeader,
  parseProcessStartHeader,
  processIdFromStartResult,
} from "./execToolModel";

const SANDBOX_WARNING =
  "⚠️ Sandbox preferred but full enforcement is unavailable (bwrap: partial); command ran unconfined.";
const SHELL_SCOPE_WARNINGS = [
  "⚠️ Worktree scope: absolute path was used in a worktree-scoped chat and resolved under active worktree '/wt': '/wt/app'",
  "⚠️ Worktree scope: shell cwd/workdir is enforced as '/wt/app'; OS confinement follows the terminal security mode",
];

function processSnapshot(title: string, processId: string): string {
  return [
    title,
    `process_id: ${processId}`,
    "short_description: Start dev server",
    "status: running",
    "mode: background",
    "service_name: <none>",
    "command: npm run dev",
    "cwd: /wt/app",
    "started_at: Some(1792930874877)",
    "exit_code: <none>",
    "",
  ].join("\n");
}

function processStartContent(processId: string): string {
  return [
    `${SANDBOX_WARNING}\n${processSnapshot("Process started", processId)}`,
    `\n${SHELL_SCOPE_WARNINGS[1].replace("shell", "process")}\n`,
    "stdout:\nready\n",
  ].join("");
}

function backgroundShellContent(processId: string): string {
  const started = [
    "Process started in background",
    `process_id: ${processId}`,
    "short_description: Run watcher",
    "status: running",
    "mode: background",
    "Use process_read or wait with a process target to retrieve output later.",
    "",
  ].join("\n");
  return `${SHELL_SCOPE_WARNINGS.join("\n")}\n${SANDBOX_WARNING}\n${started}`;
}

function serviceStartContent(processId: string): string {
  return [
    "Service started",
    `process_id: ${processId}`,
    "short_description: api service",
    "status: running",
    "mode: service",
    "service_name: api",
    "command: npm start",
    "cwd: /wt/app",
    "exit_code: <none>",
    "",
    "stdout:",
    "listening",
  ].join("\n");
}

describe("parseProcessStartHeader with real engine layouts", () => {
  test("process_start output with a sandbox warning before the banner", () => {
    const header = parseProcessStartHeader(processStartContent("exec_ps"));
    expect(header?.process_id).toBe("exec_ps");
    expect(header?.status).toBe("running");
    expect(header?.started_at).toBe("1792930874877");
    expect(header?.exit_code).toBeNull();
  });

  test("background shell output with scope and sandbox warnings before the banner", () => {
    const header = parseProcessStartHeader(backgroundShellContent("exec_bg"));
    expect(header?.process_id).toBe("exec_bg");
    expect(header?.mode).toBe("background");
    expect(header?.short_description).toBe("Run watcher");
  });

  test("shell_service start/restart output", () => {
    expect(
      processIdFromStartResult(undefined, serviceStartContent("exec_svc")),
    ).toBe("exec_svc");
  });

  test("only the key/value block after the banner is read", () => {
    const content = `${processSnapshot(
      "Process started",
      "exec_a",
    )}\nstdout:\nprocess_id: exec_b\nstatus: failed\n`;
    const header = parseProcessStartHeader(content);
    expect(header?.process_id).toBe("exec_a");
    expect(header?.status).toBe("running");
  });

  test("ordinary stdout that looks like a header is rejected", () => {
    const stdout = "process_id: exec_other\nstatus: running\nmode: background";
    expect(parseProcessStartHeader(stdout)).toBeNull();
    expect(processIdFromStartResult(undefined, stdout)).toBeNull();
    expect(execMetadataFromResult(undefined, stdout, "start")).toBeNull();
  });

  test("a banner further down the output is not a start header", () => {
    const content = `build log\n${processSnapshot(
      "Process started",
      "exec_x",
    )}`;
    expect(parseProcessStartHeader(content)).toBeNull();
  });

  test("snapshot banners are not start banners", () => {
    const read = processSnapshot("Process output", "exec_r");
    expect(parseProcessStartHeader(read)).toBeNull();
    expect(parseProcessSnapshotHeader(read)?.process_id).toBe("exec_r");
    const wait = `outcome: completed\ntarget: process\n${processSnapshot(
      "Process wait completed",
      "exec_w",
    )}`;
    expect(parseProcessSnapshotHeader(wait)?.process_id).toBe("exec_w");
  });

  test("foreground shell results are never parsed for metadata", () => {
    const content = processSnapshot("Process output", "exec_r");
    expect(execMetadataFromResult(undefined, content, "none")).toBeNull();
  });
});

describe("isProcessStartCall", () => {
  test("covers every start path and nothing else", () => {
    expect(isProcessStartCall("process_start", {})).toBe(true);
    expect(isProcessStartCall("shell", { run_in_background: true })).toBe(true);
    expect(isProcessStartCall("shell", { run_in_background: "true" })).toBe(
      true,
    );
    expect(isProcessStartCall("shell", { run_in_background: "no" })).toBe(
      false,
    );
    expect(isProcessStartCall("shell", { command: "ls" })).toBe(false);
    expect(isProcessStartCall("shell_service", { action: "start" })).toBe(true);
    expect(isProcessStartCall("shell_service", { action: "restart" })).toBe(
      true,
    );
    expect(isProcessStartCall("shell_service", { action: "status" })).toBe(
      false,
    );
    expect(isProcessStartCall("process_read", {})).toBe(false);
  });
});
