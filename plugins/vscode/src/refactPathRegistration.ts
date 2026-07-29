import { spawn } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { sharedRefactBinaryDir } from "./refactBinaryResolver";

/**
 * Registrar that adds the canonical shared refact binary directory
 * (`~/.refact/bin`) to the PATH used by future terminals.
 *
 * The implementation mirrors the Codex-style shell installer: on Unix it
 * upserts an owned block delimited by exact markers into a single shell
 * profile; on Windows it persists the directory into the user's PATH via a
 * PowerShell child process. All operations are best-effort and never throw:
 * failures are surfaced as a non-fatal result so callers can log a warning.
 */

export const REFACT_PATH_MARKER_BEGIN = "# >>> Refact installer >>>";
export const REFACT_PATH_MARKER_END = "# <<< Refact installer <<<";

export type RegisterRefactPathOutcome =
    | "added"
    | "updated"
    | "unchanged"
    | "warning";

export type RegisterRefactPathResult = {
    outcome: RegisterRefactPathOutcome;
    /** Absolute path of the profile file touched (Unix), if any. */
    file?: string;
    /** Human readable message; set for `warning` and informational outcomes. */
    message?: string;
};

export type ShellFamily = "zsh" | "bash" | "fish" | "sh";

export type RegisterRefactPathOptions = {
    homeDir?: string;
    platform?: string;
    /** Basename of the current shell, e.g. from process.env.SHELL. */
    shell?: string;
    env?: NodeJS.ProcessEnv;
    // Injectable fs seams for testing.
    readFile?: (filePath: string) => string | undefined;
    writeFile?: (filePath: string, contents: string) => void;
    mkdir?: (dirPath: string) => void;
    // Injectable Windows PATH seam for testing.
    runWindowsPathUpdate?: (dir: string) => Promise<WindowsPathUpdateResult>;
};

export type WindowsPathUpdateResult = {
    outcome: "added" | "unchanged" | "warning";
    message?: string;
};

function defaultReadFile(filePath: string): string | undefined {
    try {
        return fs.readFileSync(filePath, "utf8");
    } catch {
        return undefined;
    }
}

function defaultWriteFile(filePath: string, contents: string): void {
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(filePath, contents, "utf8");
}

function defaultMkdir(dirPath: string): void {
    fs.mkdirSync(dirPath, { recursive: true });
}

/**
 * Choose a single Codex-style shell profile file for the current platform and
 * shell. Returns the absolute path plus the shell family that owns it.
 */
export function chooseUnixProfile(
    homeDir: string,
    platform: string,
    shellName: string | undefined,
    env: NodeJS.ProcessEnv = {},
): { file: string; family: ShellFamily } {
    const shell = (shellName ?? "").trim().toLowerCase();
    const xdgConfigHome = env.XDG_CONFIG_HOME?.trim() || path.join(homeDir, ".config");
    if (shell.includes("fish")) {
        return { file: path.join(xdgConfigHome, "fish", "config.fish"), family: "fish" };
    }
    if (shell.includes("zsh")) {
        // Codex-style: login profile on macOS, rc file on Linux.
        return platform === "darwin"
            ? { file: path.join(homeDir, ".zprofile"), family: "zsh" }
            : { file: path.join(homeDir, ".zshrc"), family: "zsh" };
    }
    if (shell.includes("bash")) {
        return platform === "darwin"
            ? { file: path.join(homeDir, ".bash_profile"), family: "bash" }
            : { file: path.join(homeDir, ".bashrc"), family: "bash" };
    }
    // Fallback for unknown / POSIX shells.
    return { file: path.join(homeDir, ".profile"), family: "sh" };
}

function pathExportBody(family: ShellFamily): string {
    if (family === "fish") {
        return 'fish_add_path "$HOME/.refact/bin"';
    }
    return 'export PATH="$HOME/.refact/bin:$PATH"';
}

/**
 * Build the owned block (markers + body) for the given directory.
 */
export function refactPathBlock(dir: string, family: ShellFamily): string {
    void dir;
    return `${REFACT_PATH_MARKER_BEGIN}\n${pathExportBody(family)}\n${REFACT_PATH_MARKER_END}`;
}

type MarkerScan =
    | { kind: "none" }
    | { kind: "malformed"; reason: string }
    | { kind: "found"; start: number; end: number };

function scanMarkers(content: string): MarkerScan {
    const beginMatches = exactMarkerMatches(content, REFACT_PATH_MARKER_BEGIN);
    const endMatches = exactMarkerMatches(content, REFACT_PATH_MARKER_END);
    if (beginMatches.length === 0 && endMatches.length === 0) {
        return { kind: "none" };
    }
    if (beginMatches.length !== 1 || endMatches.length !== 1) {
        return { kind: "malformed", reason: `unbalanced installer markers (begin=${beginMatches.length}, end=${endMatches.length})` };
    }
    const start = beginMatches[0].index;
    const end = endMatches[0].index;
    if (end < start) {
        return { kind: "malformed", reason: "installer end marker precedes begin marker" };
    }
    return { kind: "found", start, end: end + endMatches[0].length };
}

function exactMarkerMatches(content: string, marker: string): Array<{ index: number; length: number }> {
    const expression = new RegExp(`^${escapeRegExp(marker)}\\r?$`, "gm");
    return Array.from(content.matchAll(expression), match => ({ index: match.index ?? 0, length: match[0].length }));
}

function escapeRegExp(value: string): string {
    return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Pure upsert: given existing profile content, produce the new content that
 * contains exactly one owned block for `dir`. Returns `undefined` for the
 * content when the block already matches (idempotent no-op), and reports a
 * malformed condition instead of mutating a file with broken markers.
 */
export function upsertRefactPathBlock(
    existingContent: string | undefined,
    dir: string,
    family: ShellFamily,
): { outcome: RegisterRefactPathOutcome; content?: string; message?: string } {
    const block = refactPathBlock(dir, family);
    const content = existingContent ?? "";
    const scan = scanMarkers(content);

    if (scan.kind === "malformed") {
        return { outcome: "warning", message: `refusing to edit profile with ${scan.reason}` };
    }

    if (scan.kind === "none") {
        const needsLeadingNewline = content.length > 0 && !content.endsWith("\n");
        const separator = content.length === 0 ? "" : (needsLeadingNewline ? "\n\n" : "\n");
        const trailing = content.length === 0 ? "\n" : "";
        return { outcome: "added", content: `${content}${separator}${block}\n${trailing}` };
    }

    // Found exactly one well-formed block; replace its body if stale.
    const existingBlock = content.slice(scan.start, scan.end);
    if (existingBlock === block) {
        return { outcome: "unchanged" };
    }
    const updated = content.slice(0, scan.start) + block + content.slice(scan.end);
    return { outcome: "updated", content: updated };
}

function registerUnixRefactPath(
    dir: string,
    options: Required<Pick<RegisterRefactPathOptions, "homeDir" | "platform">> & RegisterRefactPathOptions,
): RegisterRefactPathResult {
    const readFile = options.readFile ?? defaultReadFile;
    const writeFile = options.writeFile ?? defaultWriteFile;
    const shellName = options.shell ?? basenameOrUndefined(options.env?.SHELL);
    const { file, family } = chooseUnixProfile(options.homeDir, options.platform, shellName, options.env ?? {});

    try {
        const existing = readFile(file);
        const result = upsertRefactPathBlock(existing, dir, family);
        if (result.outcome === "warning") {
            return { outcome: "warning", file, message: result.message };
        }
        if (result.outcome === "unchanged") {
            return { outcome: "unchanged", file };
        }
        writeFile(file, result.content ?? "");
        return { outcome: result.outcome, file };
    } catch (error) {
        return { outcome: "warning", file, message: errorMessage(error) };
    }
}

function basenameOrUndefined(value: string | undefined): string | undefined {
    const trimmed = value?.trim();
    return trimmed ? path.basename(trimmed) : undefined;
}

async function registerWindowsRefactPath(
    dir: string,
    options: RegisterRefactPathOptions,
): Promise<RegisterRefactPathResult> {
    const runner = options.runWindowsPathUpdate ?? runWindowsUserPathUpdate;
    try {
        const result = await runner(dir);
        return { outcome: result.outcome, message: result.message };
    } catch (error) {
        return { outcome: "warning", message: errorMessage(error) };
    }
}

/**
 * PowerShell script that idempotently, case-insensitively appends `dir` to the
 * persisted per-user PATH environment variable. Prints ADDED / UNCHANGED so the
 * caller can classify the outcome.
 */
export function windowsPathUpdateScript(): string {
    return [
        "$ErrorActionPreference = 'Stop'",
        "$target = 'User'",
        "$encodedDir = [Console]::In.ReadToEnd()",
        "$dir = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($encodedDir))",
        "if ([string]::IsNullOrWhiteSpace($dir)) { throw 'Refact bin directory is empty' }",
        "$current = [Environment]::GetEnvironmentVariable('Path', $target)",
        "if ($null -eq $current) { $current = '' }",
        "$parts = $current -split ';' | Where-Object { $_ -ne '' }",
        "$exists = $false",
        "foreach ($p in $parts) { if ($p.TrimEnd('\\') -ieq $dir.TrimEnd('\\')) { $exists = $true } }",
        "if ($exists) { Write-Output 'UNCHANGED'; exit 0 }",
        "$sep = if ($current -eq '' -or $current.EndsWith(';')) { '' } else { ';' }",
        "$newValue = $current + $sep + $dir",
        "[Environment]::SetEnvironmentVariable('Path', $newValue, $target)",
        "Write-Output 'ADDED'",
    ].join("\n");
}

export function classifyWindowsPathUpdateOutput(stdout: string): WindowsPathUpdateResult {
    return { outcome: stdout.includes("ADDED") ? "added" : "unchanged" };
}

function runWindowsUserPathUpdate(dir: string): Promise<WindowsPathUpdateResult> {
    return new Promise(resolve => {
        const script = windowsPathUpdateScript();
        const child = spawn(
            "powershell.exe",
            ["-NoProfile", "-NonInteractive", "-Command", script],
            { stdio: ["pipe", "pipe", "pipe"] },
        );
        const stdoutChunks: Buffer[] = [];
        const stderrChunks: Buffer[] = [];
        let settled = false;
        const finish = (result: WindowsPathUpdateResult) => {
            if (settled) {
                return;
            }
            settled = true;
            clearTimeout(timer);
            resolve(result);
        };
        const timer = setTimeout(() => {
            child.kill();
            finish({ outcome: "warning", message: "powershell PATH update timed out" });
        }, 30000);
        child.stdout?.on("data", chunk => stdoutChunks.push(Buffer.from(chunk)));
        child.stderr?.on("data", chunk => stderrChunks.push(Buffer.from(chunk)));
        child.once("error", error => {
            finish({ outcome: "warning", message: errorMessage(error) });
        });
        child.once("close", code => {
            const stdout = Buffer.concat(stdoutChunks).toString("utf8").trim();
            if (code !== 0) {
                const stderr = Buffer.concat(stderrChunks).toString("utf8").trim();
                finish({ outcome: "warning", message: stderr || `powershell exited with code ${code}` });
                return;
            }
            finish(classifyWindowsPathUpdateOutput(stdout));
        });
        child.stdin?.end(Buffer.from(dir, "utf8").toString("base64"));
    });
}

function errorMessage(error: unknown): string {
    return error instanceof Error ? error.message : String(error);
}

/**
 * Register the canonical `~/.refact/bin` directory on PATH for future
 * terminals. Never throws; returns a result describing the outcome.
 */
export async function registerSharedRefactPath(
    options: RegisterRefactPathOptions = {},
): Promise<RegisterRefactPathResult> {
    const homeDir = options.homeDir ?? os.homedir();
    const platform = options.platform ?? process.platform;
    if (!homeDir) {
        return { outcome: "warning", message: "home directory is not available" };
    }
    const dir = sharedRefactBinaryDir(homeDir);

    try {
        if (platform === "win32") {
            return await registerWindowsRefactPath(dir, options);
        }
        const mkdir = options.mkdir ?? defaultMkdir;
        try {
            mkdir(dir);
        } catch {
            // Directory creation is best-effort; profile edit is what matters.
        }
        return registerUnixRefactPath(dir, { ...options, homeDir, platform });
    } catch (error) {
        return { outcome: "warning", message: errorMessage(error) };
    }
}
