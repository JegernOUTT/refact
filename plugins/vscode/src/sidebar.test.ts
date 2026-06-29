import * as assert from "assert";
import * as path from "path";
import {
    createCurrentProjectInfo,
    resolveFilePathWithinWorkspace,
} from "./sidebarPaths";
import { webviewEndpointConfig } from "./sidebarConfig";

export function runSidebarPathBoundaryTests() {
    const workspaceRoot = path.resolve("/workspace/repo");

    assert.strictEqual(
        resolveFilePathWithinWorkspace("src/index.ts", [workspaceRoot]),
        path.resolve(workspaceRoot, "src/index.ts"),
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace(path.resolve("/workspace/other/file.ts"), [workspaceRoot]),
        undefined,
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace("../other/file.ts", [workspaceRoot]),
        undefined,
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace(path.resolve("/workspace/repo2/file.ts"), [workspaceRoot]),
        undefined,
    );

    assert.strictEqual(
        resolveFilePathWithinWorkspace(path.resolve(workspaceRoot, "file.ts"), []),
        undefined,
    );

    assert.deepStrictEqual(createCurrentProjectInfo("repo", []), { name: "repo" });
}

function runWebviewEndpointConfigTests() {
    const lspUrl = "http://127.0.0.1:8488/p/PID/";
    const browserUrl = "http://machine.local:8488/p/PID/";

    assert.deepStrictEqual(webviewEndpointConfig(true, lspUrl, browserUrl), { browserUrl, lspUrl });

    const startingConfig = webviewEndpointConfig(false, lspUrl, browserUrl);
    assert.deepStrictEqual(startingConfig, { browserUrl });
    assert.strictEqual(Object.prototype.hasOwnProperty.call(startingConfig, "lspUrl"), false);

    const missingProxyConfig = webviewEndpointConfig(true, "", browserUrl);
    assert.deepStrictEqual(missingProxyConfig, { browserUrl });
    assert.strictEqual(Object.prototype.hasOwnProperty.call(missingProxyConfig, "lspUrl"), false);
}

runSidebarPathBoundaryTests();
runWebviewEndpointConfigTests();
