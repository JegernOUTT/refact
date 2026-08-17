import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import statusDotStyles from "../../../components/ui/StatusDot/StatusDot.module.css";
import type { PrivacyPolicyResponse } from "../../../services/refact/privacy";
import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { FilesPanel } from "./FilesPanel";
import { movePathToPrivacyZone } from "./fileTreeModel";

const rootPath = "/workspace";
const secretPath = `${rootPath}/.env`;

const privacyResponse: PrivacyPolicyResponse = {
  policy: {
    blocked: [],
    zones: [
      {
        name: "secrets",
        patterns: [".env*"],
        send_to: ["trusted"],
        on_shell_read: "withhold",
      },
      {
        name: "normal",
        patterns: ["**"],
        send_to: ["*"],
        on_shell_read: "withhold",
      },
    ],
    subagents: { report_declassifies: true },
    tool_access: { providers: {} },
  },
  destinations: [
    {
      id: "trusted",
      kind: "provider",
      display_name: "Trusted provider",
    },
    {
      id: "build-mcp",
      kind: "mcp",
      display_name: "Build MCP",
    },
  ],
  match_counts: { secrets: 1, normal: 0 },
  error: null,
  source_paths: ["/home/user/.config/refact/privacy.yaml"],
  has_project_overrides: false,
};

const installHandlers = () => {
  server.use(
    http.get("*/v1/files/tree", ({ request }) => {
      const path = new URL(request.url).searchParams.get("path") ?? "";
      return HttpResponse.json({
        path,
        entries:
          path === rootPath
            ? [
                {
                  name: ".env",
                  path: secretPath,
                  kind: "file",
                  size: 7,
                  privacy_zone: privacyResponse.policy.zones[0],
                },
              ]
            : [
                {
                  name: "workspace",
                  path: rootPath,
                  kind: "dir",
                  size: null,
                  privacy_zone: privacyResponse.policy.zones[1],
                },
              ],
        truncated: false,
      });
    }),
    http.get("*/v1/privacy/policy", () => HttpResponse.json(privacyResponse)),
  );
};

describe("FileTree privacy zones", () => {
  it("renders a secrets-zone dot with its allowed destinations", async () => {
    installHandlers();
    const { user } = render(<FilesPanel />);
    await user.click(
      await screen.findByRole("treeitem", { name: /workspace/i }),
    );

    const secret = await screen.findByRole("treeitem", { name: /\.env/i });
    const dot = secret.querySelector('[data-zone="secrets"]');
    expect(dot).not.toBeNull();
    expect(dot).toHaveAttribute("data-zone", "secrets");
    expect(dot).toHaveClass(statusDotStyles.warning);

    await user.hover(dot as HTMLElement);
    expect(
      await screen.findByRole("tooltip", {
        name: "Zone: secrets. Allowed destinations: Trusted provider.",
      }),
    ).toBeVisible();
  });

  it("offers zone choices from the file context menu", async () => {
    let savedPolicy: PrivacyPolicyResponse["policy"] | null = null;
    installHandlers();
    server.use(
      http.post("*/v1/privacy/policy", async ({ request }) => {
        savedPolicy = (await request.json()) as PrivacyPolicyResponse["policy"];
        return HttpResponse.json({ ...privacyResponse, policy: savedPolicy });
      }),
    );
    const { user } = render(<FilesPanel />);
    await user.click(
      await screen.findByRole("treeitem", { name: /workspace/i }),
    );
    const secret = await screen.findByRole("treeitem", { name: /\.env/i });

    await user.pointer({ target: secret, keys: "[MouseRight]" });
    const move = await screen.findByRole("menuitem", { name: "Move to zone" });
    await user.hover(move);
    const normal = await screen.findByRole("menuitem", { name: "normal" });
    expect(normal).toBeVisible();
    fireEvent.click(normal);

    await waitFor(() => expect(savedPolicy).not.toBeNull());
    const posted = savedPolicy as PrivacyPolicyResponse["policy"] | null;
    expect(posted?.zones.at(1)?.patterns).toEqual([secretPath, "**"]);
  });

  it("moves an exact file rule between zones", () => {
    const exactPolicy = {
      ...privacyResponse.policy,
      zones: [
        { ...privacyResponse.policy.zones[0], patterns: [secretPath] },
        { ...privacyResponse.policy.zones[1], patterns: ["*.ts"] },
      ],
    };
    const moved = movePathToPrivacyZone(exactPolicy, secretPath, "normal");

    expect(moved.zones).toEqual([
      { ...privacyResponse.policy.zones[0], patterns: [] },
      {
        ...privacyResponse.policy.zones[1],
        patterns: [secretPath, "*.ts"],
      },
    ]);
  });

  it("keeps privacy requests bounded for a large directory", async () => {
    let treeRequests = 0;
    let inspectRequests = 0;
    const normal = privacyResponse.policy.zones[1];
    server.use(
      http.get("*/v1/files/tree", ({ request }) => {
        treeRequests += 1;
        const path = new URL(request.url).searchParams.get("path") ?? "";
        return HttpResponse.json({
          path,
          entries:
            path === rootPath
              ? Array.from({ length: 500 }, (_, index) => ({
                  name: `file-${index}.txt`,
                  path: `${rootPath}/file-${index}.txt`,
                  kind: "file",
                  size: index,
                  privacy_zone: normal,
                }))
              : [
                  {
                    name: "workspace",
                    path: rootPath,
                    kind: "dir",
                    size: null,
                    privacy_zone: normal,
                  },
                ],
          truncated: false,
        });
      }),
      http.get("*/v1/privacy/policy", () => HttpResponse.json(privacyResponse)),
      http.post("*/v1/privacy/inspect", () => {
        inspectRequests += 1;
        return HttpResponse.json({});
      }),
    );
    const { user } = render(<FilesPanel />);
    await user.click(
      await screen.findByRole("treeitem", { name: /workspace/i }),
    );

    await waitFor(() => expect(treeRequests).toBe(2));
    expect(inspectRequests).toBe(0);
  });
});
