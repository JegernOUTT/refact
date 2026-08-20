import { describe, expect, test, beforeEach } from "vitest";
import { http, HttpResponse } from "msw";

import {
  render,
  screen,
  waitFor,
  stubResizeObserver,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import {
  defaultProjectInformationConfig,
  type ProjectInfoBlock,
} from "../../services/refact/projectInformation";
import { ProjectInformationDialog } from "./ProjectInformationDialog";

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.setPointerCapture = () => undefined;
HTMLElement.prototype.releasePointerCapture = () => undefined;
stubResizeObserver();

const config = {
  apiKey: "test",
  host: "web" as const,
  dev: true,
  themeProps: {},
  lspPort: 8001,
};

type BlockSeed = Pick<ProjectInfoBlock, "id" | "section" | "title"> &
  Partial<ProjectInfoBlock>;

const makeBlock = (seed: BlockSeed): ProjectInfoBlock => ({
  path: null,
  content: "content",
  truncated: false,
  enabled: true,
  char_count: 400,
  ...seed,
});

const PREVIEW_BLOCKS: ProjectInfoBlock[] = [
  makeBlock({ id: "system_info:0", section: "system_info", title: "System" }),
  makeBlock({
    id: "instruction_files:0",
    section: "instruction_files",
    title: "AGENTS.md",
    path: "AGENTS.md",
  }),
  makeBlock({
    id: "instruction_files:1",
    section: "instruction_files",
    title: "docs/CONTRIBUTING.md",
    path: "docs/CONTRIBUTING.md",
  }),
];

// 3 blocks * 400 chars / 4 chars-per-token
const TOTAL_TOKENS = 300;
const ONE_BLOCK_TOKENS = 100;

const totalText = (tokens: number) =>
  new RegExp(`Total: ~${tokens.toLocaleString()} tokens`);

type Counters = {
  get: number;
  save: number;
  preview: number;
};

function setUpHandlers(): Counters {
  const counters: Counters = { get: 0, save: 0, preview: 0 };

  server.use(
    http.get("*/v1/project-information", () => {
      counters.get += 1;
      return HttpResponse.json(defaultProjectInformationConfig);
    }),
    http.post("*/v1/project-information", () => {
      counters.save += 1;
      return HttpResponse.json({});
    }),
    http.post("*/v1/project-information/preview", () => {
      counters.preview += 1;
      return HttpResponse.json({ blocks: PREVIEW_BLOCKS, warnings: [] });
    }),
  );

  return counters;
}

function renderDialog() {
  return render(
    <ProjectInformationDialog
      chatId="test-chat"
      open={true}
      onOpenChange={() => undefined}
    />,
    { preloadedState: { config } },
  );
}

const settle = (ms = 700) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitForFirstPreview() {
  await screen.findByText(totalText(TOTAL_TOKENS), {}, { timeout: 3000 });
}

describe("ProjectInformationDialog", () => {
  let counters: Counters;

  beforeEach(() => {
    counters = setUpHandlers();
  });

  test("toggling a file switch is optimistic and issues no preview call", async () => {
    const { user } = renderDialog();
    await waitForFirstPreview();
    const previewsAfterMount = counters.preview;
    expect(previewsAfterMount).toBe(1);

    const fileSwitch = screen.getByRole("switch", { name: "AGENTS.md" });
    expect(fileSwitch).toHaveAttribute("aria-checked", "true");

    await user.click(fileSwitch);

    // flips immediately, without waiting for any round-trip
    expect(fileSwitch).toHaveAttribute("aria-checked", "false");
    expect(
      screen.getByText(totalText(TOTAL_TOKENS - ONE_BLOCK_TOKENS)),
    ).toBeInTheDocument();

    await settle();
    expect(counters.preview).toBe(previewsAfterMount);
  });

  test("changing a limit slider triggers exactly one debounced preview", async () => {
    const { user } = renderDialog();
    await waitForFirstPreview();
    const previewsAfterMount = counters.preview;

    const slider = screen.getAllByRole("slider")[0];
    slider.focus();
    await user.keyboard("{ArrowRight}");

    await waitFor(() => expect(counters.preview).toBe(previewsAfterMount + 1), {
      timeout: 3000,
    });

    await settle();
    expect(counters.preview).toBe(previewsAfterMount + 1);
  });

  test("saving posts the config and does not refetch it afterwards", async () => {
    const { user } = renderDialog();
    await waitForFirstPreview();
    expect(counters.get).toBe(1);

    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(counters.save).toBe(1));
    expect(await screen.findByText("Configuration saved!")).toBeInTheDocument();

    await settle();
    expect(counters.get).toBe(1);
    expect(counters.save).toBe(1);
  });

  test("a section toggle updates the token total without a network call", async () => {
    const { user } = renderDialog();
    await waitForFirstPreview();
    const previewsAfterMount = counters.preview;

    await user.click(
      screen.getByRole("switch", { name: "System Information" }),
    );

    expect(
      screen.getByText(totalText(TOTAL_TOKENS - ONE_BLOCK_TOKENS)),
    ).toBeInTheDocument();

    await settle();
    expect(counters.preview).toBe(previewsAfterMount);
    expect(counters.get).toBe(1);
    expect(counters.save).toBe(0);
  });
});
