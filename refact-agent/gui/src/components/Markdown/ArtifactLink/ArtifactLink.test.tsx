import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { stubIntersectionObserver } from "../../../utils/test-utils";
import { Markdown } from "../Markdown";
import { ToolMarkdown } from "../ToolMarkdown";
import { HTML_PREVIEW_POLICY } from "./htmlPreviewDocument";
import { parseDelimitedPreview } from "./artifactKind";
import { InternalLinkProvider } from "../../../contexts/InternalLinkContext";
import { setUpStore } from "../../../app/store";
import { updateConfig } from "../../../features/Config/configSlice";
import { ReportMarkdown } from "../../ChatContent/ToolCard/ReportToolCard";

const ARTIFACTS_DIR = "/home/u/.refact/chats/chat-1/artifacts";

function stubClipboard() {
  const writeText = vi.fn(() => Promise.resolve());
  Object.defineProperty(window.navigator, "clipboard", {
    configurable: true,
    value: { writeText },
  });
  return writeText;
}

type ResolveOverrides = {
  exists?: boolean;
  bytes?: number | null;
};

function resolveHandler(
  overrides: Partial<Record<string, ResolveOverrides>> = {},
  requests: string[] = [],
) {
  return http.get("*/v1/artifacts/resolve", ({ request }) => {
    const url = new URL(request.url);
    const chatId = url.searchParams.get("chat_id") ?? "";
    const relPath = url.searchParams.get("path") ?? "";
    requests.push(`${chatId}:${relPath}`);
    const override = overrides[relPath] ?? {};
    return HttpResponse.json({
      chat_id: chatId,
      rel_path: relPath,
      path: `${ARTIFACTS_DIR}/${relPath}`,
      artifacts_dir: ARTIFACTS_DIR,
      exists: override.exists ?? true,
      bytes: override.bytes === undefined ? 24_883 : override.bytes,
      modified_at_ms: 1_700_000_000_000,
    });
  });
}

function readHandler(content: Record<string, string>, requests: URL[] = []) {
  return http.get("*/v1/files/read", ({ request }) => {
    const url = new URL(request.url);
    requests.push(url);
    const path = url.searchParams.get("path") ?? "";
    const relPath = path.slice(ARTIFACTS_DIR.length + 1);
    if (!(relPath in content))
      return HttpResponse.json({ detail: "missing" }, { status: 404 });
    return HttpResponse.json({
      path,
      content: content[relPath],
      language: null,
      size: content[relPath].length,
      truncated: false,
      line_start: null,
      line_end: null,
      mtime_ms: 1,
    });
  });
}

function stubVisibleIntersectionObserver() {
  class VisibleObserver {
    constructor(private readonly callback: IntersectionObserverCallback) {}
    observe(target: Element) {
      this.callback(
        [{ target, isIntersecting: true } as IntersectionObserverEntry],
        this as unknown as IntersectionObserver,
      );
    }
    unobserve() {
      return undefined;
    }
    disconnect() {
      return undefined;
    }
    takeRecords() {
      return [];
    }
  }
  vi.stubGlobal("IntersectionObserver", VisibleObserver);
}

const link = (rel: string, text = rel) =>
  `[${text}](refact://artifact/chat-1/${rel})`;

describe("artifact links in Markdown", () => {
  beforeEach(() => {
    stubVisibleIntersectionObserver();
    server.use(readHandler({}));
  });
  afterEach(() => {
    stubIntersectionObserver();
  });

  it("does not resolve artifacts before they become visible", async () => {
    stubIntersectionObserver();
    const requests: string[] = [];
    server.use(resolveHandler({}, requests));
    render(
      <Markdown>
        {`${link("reports/q3.html", "Q3 report")}\n\nSee ${link("d.csv")}.`}
      </Markdown>,
    );
    const card = screen.getByTestId("artifact-card");
    expect(within(card).getByText("Q3 report")).toBeInTheDocument();
    expect(
      within(card).getByText("HTML · reports/q3.html"),
    ).toBeInTheDocument();
    expect(screen.getByTestId("artifact-chip")).toHaveTextContent("d.csv");
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(requests).toEqual([]);
  });

  it("shares one resolve request between repeated links", async () => {
    const requests: string[] = [];
    server.use(resolveHandler({}, requests));
    render(
      <Markdown>
        {`${link("q3.bin")}\n\nInline ${link("q3.bin")} again ${link(
          "q3.bin",
        )}.`}
      </Markdown>,
    );
    const card = await screen.findByTestId("artifact-card");
    await waitFor(() => expect(card).toHaveAttribute("data-state", "ready"));
    expect(requests).toEqual(["chat-1:q3.bin"]);
  });

  it("renders a standalone link as a card and an inline link as a chip", async () => {
    server.use(resolveHandler());
    render(
      <Markdown canHaveInteractiveElements>
        {`${link("reports/q3.html", "Q3 report")}\n\nSee ${link(
          "data.csv",
        )} for details.`}
      </Markdown>,
    );
    const card = await screen.findByTestId("artifact-card");
    expect(within(card).getByText("Q3 report")).toBeInTheDocument();
    await waitFor(() =>
      expect(
        within(card).getByText("HTML · 24.3 KB · reports/q3.html"),
      ).toBeInTheDocument(),
    );
    const chip = screen.getByTestId("artifact-chip");
    expect(chip).toHaveTextContent("data.csv");
    expect(chip.parentElement).toHaveTextContent("See data.csv for details.");
    expect(screen.getAllByTestId("artifact-card")).toHaveLength(1);
  });

  it("lifts a <br>-separated line into a card and keeps the surrounding text", async () => {
    server.use(resolveHandler());
    const { container } = render(
      <Markdown>{`Here is the report:\n${link("q3.html")}\nThanks!`}</Markdown>,
    );
    const card = await screen.findByTestId("artifact-card");
    const paragraphs = Array.from(container.querySelectorAll("p")).map(
      (node) => node.textContent?.trim(),
    );
    expect(paragraphs).toEqual(["Here is the report:", "Thanks!"]);
    const order = Array.from(
      container.querySelectorAll("p, [data-testid=artifact-card]"),
    );
    expect(order.indexOf(card)).toBe(1);
    expect(screen.queryByTestId("artifact-chip")).not.toBeInTheDocument();
  });

  it("turns a list item that only holds an artifact link into a card", async () => {
    server.use(resolveHandler());
    render(
      <Markdown canHaveInteractiveElements>
        {`- ${link("a.md")}\n- ${link("b.md")} and more`}
      </Markdown>,
    );
    expect(await screen.findAllByTestId("artifact-card")).toHaveLength(1);
    expect(screen.getAllByTestId("artifact-chip")).toHaveLength(1);
  });

  it("splits tight list item lines so a standalone link line becomes a card", async () => {
    server.use(resolveHandler());
    const { container } = render(
      <Markdown canHaveInteractiveElements>
        {`- Intro\n  ${link("a.txt", "report")}\n  Tail ${link(
          "b.csv",
        )}\n- Second`}
      </Markdown>,
    );
    const card = await screen.findByTestId("artifact-card");
    const items = container.querySelectorAll("li");
    expect(items).toHaveLength(2);
    expect(items[0]).toContainElement(card);
    expect(items[0].firstChild?.textContent?.trim()).toBe("Intro");
    expect(within(items[0]).getByTestId("artifact-chip")).toHaveTextContent(
      "b.csv",
    );
    expect(items[0].textContent).toContain("Tail");
    expect(screen.getAllByTestId("artifact-card")).toHaveLength(1);
    expect(screen.getAllByTestId("artifact-chip")).toHaveLength(1);
  });

  it("renders plain text and issues no requests when inert or non-interactive", async () => {
    const requests: string[] = [];
    server.use(resolveHandler({}, requests));
    const text = `${link("a.html", "Alone")}\n\nInline ${link("b.csv", "B")}`;
    render(
      <>
        <Markdown inert>{text}</Markdown>
        <Markdown canHaveInteractiveElements={false}>{text}</Markdown>
      </>,
    );
    expect(screen.getAllByText("Alone")).toHaveLength(2);
    expect(screen.queryByTestId("artifact-card")).not.toBeInTheDocument();
    expect(screen.queryByTestId("artifact-chip")).not.toBeInTheDocument();
    expect(screen.queryAllByRole("link")).toHaveLength(0);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(requests).toEqual([]);
  });

  it("shows a missing artifact as not clickable", async () => {
    server.use(resolveHandler({ "gone.html": { exists: false, bytes: null } }));
    render(<Markdown canHaveInteractiveElements>{link("gone.html")}</Markdown>);
    const card = await screen.findByTestId("artifact-card");
    await waitFor(() => expect(card).toHaveAttribute("data-state", "missing"));
    expect(within(card).getByText(/File not found/)).toBeInTheDocument();
    expect(
      within(card).getByRole("button", { name: "Open gone.html" }),
    ).toBeDisabled();
  });

  it("shows an error with a working retry", async () => {
    let calls = 0;
    server.use(
      http.get("*/v1/artifacts/resolve", ({ request }) => {
        calls += 1;
        if (calls === 1)
          return HttpResponse.json({ detail: "boom" }, { status: 500 });
        const url = new URL(request.url);
        return HttpResponse.json({
          chat_id: url.searchParams.get("chat_id"),
          rel_path: url.searchParams.get("path"),
          path: `${ARTIFACTS_DIR}/x.bin`,
          artifacts_dir: ARTIFACTS_DIR,
          exists: true,
          bytes: 10,
          modified_at_ms: 1,
        });
      }),
    );
    const { user } = render(
      <Markdown canHaveInteractiveElements>{link("x.bin")}</Markdown>,
    );
    const card = await screen.findByTestId("artifact-card");
    await within(card).findByText(/Artifact unavailable/);
    await user.click(within(card).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(card).toHaveAttribute("data-state", "ready"));
    expect(within(card).getByText("No preview")).toBeInTheDocument();
  });

  it("rejects a resolve response that does not echo the request", async () => {
    server.use(
      http.get("*/v1/artifacts/resolve", () =>
        HttpResponse.json({
          chat_id: "other-chat",
          rel_path: "a.txt",
          path: "/tmp/a.txt",
          artifacts_dir: "/tmp",
          exists: true,
          bytes: 1,
          modified_at_ms: 1,
        }),
      ),
    );
    render(<Markdown canHaveInteractiveElements>{link("a.txt")}</Markdown>);
    const card = await screen.findByTestId("artifact-card");
    await waitFor(() => expect(card).toHaveAttribute("data-state", "error"));
  });

  it("opens the resolved absolute path in the files panel and never uses internal link handling", async () => {
    server.use(resolveHandler());
    const onInternalLink = vi.fn(() => Promise.resolve(true));
    const { user, store } = render(
      <InternalLinkProvider onInternalLink={onInternalLink}>
        <Markdown canHaveInteractiveElements>
          {`${link("q3.html")}\n\nInline ${link("d.csv")}.`}
        </Markdown>
      </InternalLinkProvider>,
    );
    const open = await screen.findByRole("button", { name: "Open q3.html" });
    await waitFor(() => expect(open).toBeEnabled());
    await user.click(open);
    expect(store.getState().filesPanel.viewerTarget?.path).toBe(
      `${ARTIFACTS_DIR}/q3.html`,
    );

    const chip = screen.getByTestId("artifact-chip");
    await waitFor(() => expect(chip).toHaveAttribute("aria-disabled", "false"));
    await user.click(chip);
    expect(store.getState().filesPanel.viewerTarget?.path).toBe(
      `${ARTIFACTS_DIR}/d.csv`,
    );
    expect(onInternalLink).not.toHaveBeenCalled();
  });

  it("renders a disabled card and chip when the host cannot open files", async () => {
    server.use(resolveHandler());
    const store = setUpStore();
    store.dispatch(
      updateConfig({
        host: "web",
        capabilities: { openFileInApp: false, openFileInIde: false },
      }),
    );
    const { user } = render(
      <Markdown>{`${link("q3.html")}\n\nInline ${link("d.csv")}.`}</Markdown>,
      { store },
    );
    const card = await screen.findByTestId("artifact-card");
    await waitFor(() => expect(card).toHaveAttribute("data-state", "ready"));
    expect(
      within(card).getByRole("button", { name: "Open q3.html" }),
    ).toBeDisabled();
    const chip = screen.getByTestId("artifact-chip");
    expect(chip).toHaveAttribute("aria-disabled", "true");
    await user.click(chip);
    expect(store.getState().filesPanel.viewerTarget).toBeNull();
  });

  it("copies the path without opening", async () => {
    server.use(resolveHandler());
    const { user, store } = render(
      <Markdown canHaveInteractiveElements>{link("q3.bin")}</Markdown>,
    );
    const writeText = stubClipboard();
    const button = await screen.findByRole("button", { name: "Copy path" });
    await user.click(button);
    expect(writeText).toHaveBeenCalledWith(`${ARTIFACTS_DIR}/q3.bin`);
    expect(store.getState().filesPanel.viewerTarget).toBeNull();
  });

  it("previews HTML in a sandboxed, CSP-pinned, scaled iframe", async () => {
    stubVisibleIntersectionObserver();
    server.use(
      resolveHandler(),
      readHandler({ "page.html": "<h1>Hello</h1>" }),
    );
    render(<Markdown canHaveInteractiveElements>{link("page.html")}</Markdown>);
    const frame = await screen.findByTitle("Preview of page.html");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts");
    expect(frame.getAttribute("sandbox")).not.toContain("allow-same-origin");
    expect(frame.getAttribute("srcdoc")).toContain(HTML_PREVIEW_POLICY);
    expect(frame.getAttribute("srcdoc")).toContain("<h1>Hello</h1>");
  });

  it("previews SVG only through an image data URL", async () => {
    stubVisibleIntersectionObserver();
    const svg = '<svg xmlns="http://www.w3.org/2000/svg"><circle r="4"/></svg>';
    server.use(resolveHandler(), readHandler({ "chart.svg": svg }));
    const { container } = render(
      <Markdown canHaveInteractiveElements>{link("chart.svg")}</Markdown>,
    );
    const image = await screen.findByTestId("artifact-preview-image");
    expect(image.getAttribute("src")).toBe(
      `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`,
    );
    expect(container.querySelector('circle[r="4"]')).toBeNull();
  });

  it("previews CSV as a small table using a line range", async () => {
    stubVisibleIntersectionObserver();
    const reads: URL[] = [];
    server.use(
      resolveHandler(),
      readHandler(
        { "data.csv": 'name,note\nalpha,"a, b"\nbeta,"say ""hi"""' },
        reads,
      ),
    );
    render(<Markdown canHaveInteractiveElements>{link("data.csv")}</Markdown>);
    const table = await screen.findByTestId("artifact-preview-table");
    expect(
      within(table).getByRole("columnheader", { name: "name" }),
    ).toBeInTheDocument();
    expect(
      within(table).getByRole("cell", { name: "a, b" }),
    ).toBeInTheDocument();
    expect(
      within(table).getByRole("cell", { name: 'say "hi"' }),
    ).toBeInTheDocument();
    expect(reads[0].searchParams.get("line_start")).toBe("1");
    expect(reads[0].searchParams.get("line_end")).toBe("24");
  });

  it("renders markdown excerpts inertly without nested cards or requests", async () => {
    stubVisibleIntersectionObserver();
    const resolves: string[] = [];
    server.use(
      resolveHandler({}, resolves),
      readHandler({
        "notes.md": `# Notes\n\n${link(
          "nested.html",
          "Nested",
        )}\n\nInline ${link("other.csv", "Other")}`,
      }),
    );
    render(<Markdown canHaveInteractiveElements>{link("notes.md")}</Markdown>);
    const excerpt = await screen.findByTestId("artifact-preview-text");
    expect(within(excerpt).getByText("Notes")).toBeInTheDocument();
    expect(within(excerpt).getByText("Nested")).toBeInTheDocument();
    expect(screen.getAllByTestId("artifact-card")).toHaveLength(1);
    expect(screen.queryByTestId("artifact-chip")).not.toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(resolves).toEqual(["chat-1:notes.md"]);
  });

  it("renders markdown excerpts without remote images or executable fences", async () => {
    stubVisibleIntersectionObserver();
    const images: string[] = [];
    server.use(
      resolveHandler(),
      readHandler({
        "notes.md": [
          "# Notes",
          "![x](https://example.test/i.png)",
          "```html",
          "<p>hello</p>",
          "```",
          "```mermaid",
          "graph TD; A-->B",
          "```",
        ].join("\n"),
      }),
      http.get("https://example.test/*", ({ request }) => {
        images.push(request.url);
        return new HttpResponse(null, { status: 404 });
      }),
    );
    const { container } = render(
      <Markdown canHaveInteractiveElements>{link("notes.md")}</Markdown>,
    );
    const excerpt = await screen.findByTestId("artifact-preview-text");
    await waitFor(() => expect(excerpt).toHaveTextContent("<p>hello</p>"));
    expect(excerpt).toHaveTextContent("graph TD; A-->B");
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(
      container.querySelector('img[src="https://example.test/i.png"]'),
    ).toBeNull();
    expect(excerpt.querySelector("img")).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
    expect(images).toEqual([]);
  });

  it("renders tool markdown excerpts without remote images or executable fences", async () => {
    stubVisibleIntersectionObserver();
    server.use(
      resolveHandler(),
      readHandler({
        "notes.md": "![x](https://example.test/i.png)\n```html\n<b>hi</b>\n```",
      }),
    );
    const { container } = render(
      <ToolMarkdown>{link("notes.md")}</ToolMarkdown>,
    );
    const excerpt = await screen.findByTestId("artifact-preview-text");
    await waitFor(() => expect(excerpt).toHaveTextContent("<b>hi</b>"));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(excerpt.querySelector("img")).toBeNull();
    expect(container.querySelector("iframe")).toBeNull();
  });

  it("skips previews above the text size cap", async () => {
    stubVisibleIntersectionObserver();
    const reads: URL[] = [];
    server.use(
      resolveHandler({ "big.log": { bytes: 3 * 1024 * 1024 } }),
      readHandler({ "big.log": "x" }, reads),
    );
    render(<Markdown canHaveInteractiveElements>{link("big.log")}</Markdown>);
    await screen.findByText("Log · 3 MB · big.log");
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(reads).toEqual([]);
    expect(screen.queryByTestId("artifact-preview-text")).toBeNull();
  });
});

describe("artifact links in ToolMarkdown", () => {
  afterEach(() => {
    stubIntersectionObserver();
  });

  it("renders artifact cards in subagent reports", async () => {
    stubVisibleIntersectionObserver();
    server.use(resolveHandler(), readHandler({ "summary.txt": "done" }));
    render(
      <ReportMarkdown
        markdown={`# Report\n\n${link("summary.txt")}\n\nAlso ${link(
          "d.csv",
        )}.`}
      />,
    );
    expect(await screen.findByTestId("artifact-card")).toBeInTheDocument();
    expect(screen.getByTestId("artifact-chip")).toHaveTextContent("d.csv");
  });

  it("keeps artifact links but still strips other disallowed schemes", async () => {
    stubVisibleIntersectionObserver();
    server.use(resolveHandler(), readHandler({ "out.txt": "line one" }));
    render(
      <ToolMarkdown>
        {`${link("out.txt", "Output")}\n\nInline ${link(
          "x.csv",
        )} and [bad](javascript:alert(1)) and [other](refact://chat/abc).`}
      </ToolMarkdown>,
    );
    expect(await screen.findByTestId("artifact-card")).toBeInTheDocument();
    expect(
      await screen.findByTestId("artifact-preview-text"),
    ).toHaveTextContent("line one");
    expect(screen.getByTestId("artifact-chip")).toHaveTextContent("x.csv");
    const bad = screen.getByText("bad").closest("a");
    const other = screen.getByText("other").closest("a");
    expect(bad?.getAttribute("href") ?? "").toBe("");
    expect(other?.getAttribute("href") ?? "").toBe("");
  });
});

describe("parseDelimitedPreview", () => {
  it("caps rows and columns and handles quotes", () => {
    const csv = Array.from({ length: 10 }, (_, row) =>
      Array.from({ length: 9 }, (_, col) => `r${row}c${col}`).join(","),
    ).join("\r\n");
    const rows = parseDelimitedPreview(csv, ",");
    expect(rows).toHaveLength(6);
    expect(rows[0]).toHaveLength(6);
    expect(parseDelimitedPreview('a\t"b\nc"\td', "\t")).toEqual([
      ["a", "b\nc", "d"],
    ]);
  });
});
