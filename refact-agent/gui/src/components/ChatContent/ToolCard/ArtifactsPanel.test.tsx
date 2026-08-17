import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Theme } from "@radix-ui/themes";
import { describe, expect, test } from "vitest";

import { render as renderWithProviders } from "../../../utils/test-utils";
import { ArtifactsPanel } from "./ArtifactsPanel";

const artifacts = [
  {
    artifact: {
      kind: "image",
      mime: "image/png",
      data: "aW1hZ2U=",
      width: 1280,
      height: 720,
      bytes: 2_048,
    },
  },
  {
    artifact: {
      kind: "pdf",
      mime: "application/pdf",
      path: "/tmp/refact-browser/report.pdf",
      bytes: 4_096,
      page_count: 3,
      data: null,
    },
  },
];

const downloads = [
  {
    guid: "download-1",
    url: "https://example.com/report.csv",
    frame_id: "frame-1",
    suggested_filename: "report.csv",
    local_path: "/tmp/refact-browser/report.csv",
    received_bytes: 1_024,
    total_bytes: 1_024,
    state: "completed",
  },
  {
    guid: "download-2",
    url: "https://example.com/canceled.zip",
    frame_id: "frame-1",
    suggested_filename: "canceled.zip",
    local_path: "/tmp/refact-browser/canceled.zip",
    received_bytes: 512,
    total_bytes: 4_096,
    state: "canceled",
  },
];

function renderPanel(nextArtifacts: unknown, nextDownloads: unknown) {
  return renderWithProviders(
    <Theme>
      <ArtifactsPanel artifacts={nextArtifacts} downloads={nextDownloads} />
    </Theme>,
  );
}

describe("ArtifactsPanel", () => {
  test("groups mixed artifacts, opens screenshots, and distinguishes failed downloads", async () => {
    const user = userEvent.setup();
    renderPanel(artifacts, downloads);

    const trigger = screen.getByRole("button", {
      name: "Artifacts — 1 screenshot, 1 PDF, 2 downloads",
    });
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("report.pdf")).not.toBeInTheDocument();

    await user.click(trigger);

    expect(screen.getByText("Screenshots (1)")).toBeInTheDocument();
    expect(screen.getByText("1280×720 · 2.0 KB")).toBeInTheDocument();
    expect(screen.getByText("PDFs (1)")).toBeInTheDocument();
    expect(screen.getByText("report.pdf")).toBeInTheDocument();
    expect(screen.getByText("4.0 KB")).toBeInTheDocument();
    expect(screen.getByText("3 pages")).toBeInTheDocument();
    expect(screen.getByText("Downloads (2)")).toBeInTheDocument();
    expect(screen.getByText("report.csv")).toBeInTheDocument();
    expect(screen.getByText("Completed")).toBeInTheDocument();
    expect(screen.getByText("canceled.zip")).toBeInTheDocument();
    expect(screen.getByText("Canceled")).toBeInTheDocument();
    expect(screen.getByTestId("download-1")).toHaveAttribute(
      "data-status",
      "error",
    );

    const screenshot = screen.getByRole("button", {
      name: "Open image: Screenshot 1",
    });
    await user.click(screenshot);
    expect(
      screen.getByRole("application", { name: "Image pan and zoom" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Screenshot 1" })).toHaveAttribute(
      "src",
      "data:image/png;base64,aW1hZ2U=",
    );

    await user.keyboard("{Escape}");
    expect(
      screen.getByRole("link", { name: "Open PDF report.pdf" }),
    ).toHaveAttribute("href", "file:///tmp/refact-browser/report.pdf");
    expect(
      screen.getByRole("link", { name: "Open download report.csv" }),
    ).toHaveAttribute("href", "file:///tmp/refact-browser/report.csv");
  });

  test("renders nothing for empty or malformed data", () => {
    const { rerender } = renderPanel([], []);
    expect(screen.queryByTestId("artifacts-panel")).not.toBeInTheDocument();

    rerender(
      <Theme>
        <ArtifactsPanel
          artifacts={[
            null,
            { artifact: { kind: "image", mime: "image/png" } },
            { artifact: { kind: "pdf", path: 42 } },
          ]}
          downloads={[null, { state: "completed" }]}
        />
      </Theme>,
    );
    expect(screen.queryByTestId("artifacts-panel")).not.toBeInTheDocument();
  });

  test("ignores malformed entries while rendering valid partial results", async () => {
    const user = userEvent.setup();
    renderPanel(
      [artifacts[0], { artifact: { kind: "pdf", path: null } }],
      [{ state: "failed" }],
    );

    const trigger = screen.getByRole("button", {
      name: "Artifacts — 1 screenshot",
    });
    await user.click(trigger);

    expect(screen.getByText("1280×720 · 2.0 KB")).toBeInTheDocument();
    expect(screen.queryByText(/PDFs/)).not.toBeInTheDocument();
    expect(screen.queryByText(/Downloads/)).not.toBeInTheDocument();
  });
});
