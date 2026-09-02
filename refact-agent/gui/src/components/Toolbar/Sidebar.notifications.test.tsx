import { afterEach, describe, expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";

import { render, screen } from "../../utils/test-utils";
import type { RootState } from "../../app/store";
import { Toolbar } from "./Toolbar";

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function renderToolbarAndOpenEngineStatus(options: {
  preloadedState: Partial<RootState>;
}) {
  render(<Toolbar activeTab={{ type: "dashboard" }} />, options);
  await userEvent.click(screen.getByRole("button", { name: "Engine status" }));
}

describe("Toolbar", () => {
  it("opens the current origin as an external browser link in relative mode", async () => {
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];
    const open = vi.spyOn(window, "open").mockReturnValue(null);

    await renderToolbarAndOpenEngineStatus({
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          lspUrl: "http://127.0.0.1:8765/v1/ping/Refact",
          dev: true,
          themeProps: {},
        },
      },
    });

    const engineLink = await screen.findByRole("link", {
      name: `Engine URL ${window.location.origin}`,
    });
    expect(engineLink).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Open Chat in Browser"),
    ).not.toBeInTheDocument();

    await userEvent.click(engineLink);

    expect(open).toHaveBeenCalledWith(
      window.location.origin,
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("prefers an mDNS browser URL over localhost", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [
      "http://192.168.1.42:8765",
      "http://workstation.local:8765",
    ];

    await renderToolbarAndOpenEngineStatus({
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8765,
          lspUrl: "http://localhost:8765/v1/ping/Refact",
          engineServed: true,
          themeProps: {},
        },
      },
    });

    expect(
      await screen.findByLabelText("Engine URL http://workstation.local:8765"),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByLabelText("Engine URL http://workstation.local:8765"),
    );

    expect(open).toHaveBeenCalledWith(
      "http://workstation.local:8765",
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("uses a LAN browser URL when mDNS is not available", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];

    await renderToolbarAndOpenEngineStatus({
      preloadedState: {
        config: {
          host: "vscode",
          lspPort: 8765,
          lspUrl: "http://127.0.0.1:8765/v1/ping/Refact",
          browserUrl: "http://192.168.1.42:8765/",
          themeProps: {},
        },
      },
    });

    expect(
      await screen.findByLabelText("Engine URL http://192.168.1.42:8765"),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByLabelText("Engine URL http://192.168.1.42:8765"),
    );

    expect(open).toHaveBeenCalledWith(
      "http://192.168.1.42:8765",
      "_blank",
      "noopener,noreferrer",
    );
  });

  it("opens sanitized standalone URLs without stale v1 paths", async () => {
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ = [];

    await renderToolbarAndOpenEngineStatus({
      preloadedState: {
        config: {
          host: "web",
          lspPort: 8001,
          lspUrl: "https://example.com/refact/v1/ping/Refact",
          themeProps: {},
        },
      },
    });

    expect(
      await screen.findByLabelText("Engine URL https://example.com/refact"),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByLabelText("Engine URL https://example.com/refact"),
    );

    expect(open).toHaveBeenCalledWith(
      "https://example.com/refact",
      "_blank",
      "noopener,noreferrer",
    );
  });
});
