import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { render } from "../../utils/test-utils";
import {
  useGetBrowserSettingsQuery,
  useSaveBrowserSettingsMutation,
} from "../../services/refact/browserSettings";
import type { BrowserSettings } from "../../services/refact/browserSettings";
import { BrowserSettingsSection } from "./BrowserSettingsSection";

vi.mock("../../services/refact/browserSettings", () => ({
  useGetBrowserSettingsQuery: vi.fn(),
  useSaveBrowserSettingsMutation: vi.fn(),
}));

const browserSettings: BrowserSettings = {
  launch: {
    chrome_path: "/usr/bin/chromium",
    headless: false,
    chromium_sandbox: true,
    ignore_https_errors: true,
    mask_passwords: true,
    extra_args: ["--disable-gpu", "--lang=en"],
    downloads_dir: "/tmp/downloads",
    proxy_server: "http://proxy.test",
    proxy_bypass: "localhost",
  },
  lifecycle: {
    idle_timeout_secs: 600,
    evict_idle_attached: false,
    monitor_interval_secs: 10,
    relaunch_settle_ms: 800,
  },
  viewport: {
    desktop: { width: 1440, height: 900, scale_factor: 2 },
    mobile: { width: 390, height: 844, scale_factor: 3 },
    tablet: { width: 834, height: 1112, scale_factor: 2 },
  },
  timing: {
    default_wait_timeout_ms: 5000,
    max_wait_timeout_ms: 60000,
    default_poll_interval_ms: 200,
  },
  capture: {
    default_aria_snapshot_chars: 20000,
    max_aria_snapshot_chars: 100000,
    max_dom_snapshot_chars: 100000,
    max_inline_snapshot_bytes: 6144,
    max_extract_links: 500,
    max_extract_table_rows: 100,
    default_all_texts: 50,
  },
};

const saveBrowserSettings = vi.fn(
  (_settings: BrowserSettings): { unwrap: () => Promise<BrowserSettings> } => ({
    unwrap: () => Promise.resolve(browserSettings),
  }),
);

beforeEach(() => {
  vi.clearAllMocks();
  (useGetBrowserSettingsQuery as ReturnType<typeof vi.fn>).mockReturnValue({
    data: browserSettings,
    error: undefined,
    isFetching: false,
  });
  (useSaveBrowserSettingsMutation as ReturnType<typeof vi.fn>).mockReturnValue([
    saveBrowserSettings,
    { isLoading: false },
  ]);
});

describe("BrowserSettingsSection", () => {
  it("renders fetched values", () => {
    render(<BrowserSettingsSection />);

    expect(screen.getByRole("textbox", { name: "Chrome path" })).toHaveValue(
      "/usr/bin/chromium",
    );
    expect(
      screen.getByRole("spinbutton", { name: "Desktop width" }),
    ).toHaveValue(1440);
    expect(
      screen.getByRole("textbox", { name: "Extra arguments" }),
    ).toHaveValue("--disable-gpu\n--lang=en");
  });

  it("edits a field and saves the typed settings", async () => {
    const user = userEvent.setup();
    render(<BrowserSettingsSection />);

    const timeout = screen.getByRole("spinbutton", {
      name: "Idle timeout seconds",
    });
    await user.clear(timeout);
    await user.type(timeout, "900");
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(saveBrowserSettings).toHaveBeenCalledWith({
      ...browserSettings,
      lifecycle: { ...browserSettings.lifecycle, idle_timeout_secs: 900 },
    });
  });

  it("renders a backend validation error", async () => {
    const user = userEvent.setup();
    saveBrowserSettings.mockReturnValueOnce({
      unwrap: () => Promise.reject({ data: "Idle timeout is invalid" }),
    });
    render(<BrowserSettingsSection />);

    await user.click(screen.getByRole("switch", { name: "Headless" }));
    await user.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not save browser settings: Idle timeout is invalid",
    );
  });
});
