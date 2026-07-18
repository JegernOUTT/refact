export type IdeLogLevel = "error" | "warn" | "info" | "debug";

export type IdeLogLine = {
    at: number;
    level: IdeLogLevel;
    message: string;
};

const MAX_LOG_ENTRIES = 500;
const MAX_MESSAGE_LENGTH = 600;
const buffer: IdeLogLine[] = [];
let capturing = false;
let uninstaller: (() => void) | undefined;

function stringifyPart(part: unknown): string {
    if (typeof part === "string") {
        return part;
    }
    if (part instanceof Error) {
        return `${part.name}: ${part.message}`;
    }
    if (typeof part === "object" && part !== null) {
        try {
            return JSON.stringify(part);
        } catch {
            return String(part);
        }
    }
    return String(part);
}

export function recordIdeLog(level: IdeLogLevel, parts: unknown[]) {
    const message = parts.map(stringifyPart).join(" ").trim().slice(0, MAX_MESSAGE_LENGTH);
    if (!message) {
        return;
    }
    buffer.push({ at: Date.now(), level, message });
    if (buffer.length > MAX_LOG_ENTRIES) {
        buffer.splice(0, buffer.length - MAX_LOG_ENTRIES);
    }
}

export function getIdeLogSnapshot(limit: number): IdeLogLine[] {
    const normalizedLimit = Number.isFinite(limit)
        ? Math.min(MAX_LOG_ENTRIES, Math.max(0, Math.floor(limit)))
        : 0;
    return buffer.slice(Math.max(0, buffer.length - normalizedLimit));
}

export function installIdeConsoleCapture(): () => void {
    if (uninstaller) {
        return uninstaller;
    }

    const originalLog = console.log;
    const originalInfo = console.info;
    const originalWarn = console.warn;
    const originalError = console.error;
    const originalDebug = console.debug;

    console.log = (...parts: unknown[]) => {
        originalLog.apply(console, parts);
        if (!capturing) {
            capturing = true;
            try {
                recordIdeLog("info", parts);
            } finally {
                capturing = false;
            }
        }
    };
    console.info = (...parts: unknown[]) => {
        originalInfo.apply(console, parts);
        if (!capturing) {
            capturing = true;
            try {
                recordIdeLog("info", parts);
            } finally {
                capturing = false;
            }
        }
    };
    console.warn = (...parts: unknown[]) => {
        originalWarn.apply(console, parts);
        if (!capturing) {
            capturing = true;
            try {
                recordIdeLog("warn", parts);
            } finally {
                capturing = false;
            }
        }
    };
    console.error = (...parts: unknown[]) => {
        originalError.apply(console, parts);
        if (!capturing) {
            capturing = true;
            try {
                recordIdeLog("error", parts);
            } finally {
                capturing = false;
            }
        }
    };
    console.debug = (...parts: unknown[]) => {
        originalDebug.apply(console, parts);
        if (!capturing) {
            capturing = true;
            try {
                recordIdeLog("debug", parts);
            } finally {
                capturing = false;
            }
        }
    };

    uninstaller = () => {
        console.log = originalLog;
        console.info = originalInfo;
        console.warn = originalWarn;
        console.error = originalError;
        console.debug = originalDebug;
        uninstaller = undefined;
    };

    return uninstaller;
}
