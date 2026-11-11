import { beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { fireEvent, screen } from "@testing-library/react";
import { render } from "../../utils/test-utils";
import { Markdown } from "./Markdown";
import { ArtifactBlock } from "./ArtifactBlock";
import { ArtifactPolicyProvider } from "./ArtifactPolicyContext";
import { reviewArtifactHtml, reviewThemePayload } from "./artifactPolicy";

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
});

const policy = { mode: "review-seamless" } as const;
const markdown = (code: string) => `\`\`\`html\n${code}\n\`\`\``;

describe("Review artifact policy", () => {
  it("prepends restrictive CSP and removes base and refresh elements", () => {
    const doc = new DOMParser().parseFromString(
      reviewArtifactHtml(
        '<html><head><base href="https://evil.example/"><meta http-equiv=" REFRESH " content="0;url=https://evil.example"></head><body><script>1</script></body></html>',
      ),
      "text/html",
    );
    const csp = doc.head.firstElementChild;
    expect(csp?.getAttribute("http-equiv")).toBe("Content-Security-Policy");
    const content = csp?.getAttribute("content") ?? "";
    for (const directive of [
      "default-src 'none'",
      "connect-src 'none'",
      "font-src 'none'",
      "frame-src 'none'",
      "form-action 'none'",
      "base-uri 'none'",
    ])
      expect(content).toContain(directive);
    expect(doc.querySelector("base")).toBeNull();
    expect(
      Array.from(doc.querySelectorAll("meta")).some(
        (node) =>
          node.getAttribute("http-equiv")?.trim().toLowerCase() === "refresh",
      ),
    ).toBe(false);
  });

  it("automatically renders one borderless native frame without preview chrome", async () => {
    const { container } = render(
      <ArtifactPolicyProvider value={policy}>
        <Markdown>
          {markdown("<button>counter</button><script>window.x=1</script>")}
        </Markdown>
      </ArtifactPolicyProvider>,
    );
    const frame = await screen.findByTitle("Tour visual");
    expect(container.querySelectorAll("iframe")).toHaveLength(1);
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame.getAttribute("referrerpolicy")).toBe("no-referrer");
    expect(frame.getAttribute("srcdoc")).toContain("window.x=1");
    expect(frame.getAttribute("srcdoc")).toContain("background: transparent");
    expect(screen.queryByRole("button")).toBeNull();
    expect(container.textContent).not.toContain("HTML Preview");
  });

  it("validates source, generation and finite indices, coalesces and drops stale bindings", async () => {
    const reveal = vi.fn();
    const view = (code: string) => (
      <ArtifactPolicyProvider value={{ ...policy, onRevealAnchor: reveal }}>
        <Markdown>{markdown(code)}</Markdown>
      </ArtifactPolicyProvider>
    );
    const { rerender } = render(view("<p>one</p>"));
    const frame = await screen.findByTitle<HTMLIFrameElement>("Tour visual");
    const channel = /var channel = "([^"]+)"/.exec(frame.srcdoc)?.[1];
    expect(channel).toBeTruthy();
    const send = (
      index: unknown,
      source = frame.contentWindow,
      key = channel,
    ) => {
      fireEvent(
        window,
        new MessageEvent("message", {
          source,
          data: { type: "refact-review-reveal-anchor", channel: key, index },
        }),
      );
    };
    vi.useFakeTimers();
    send(0, window);
    send(0, frame.contentWindow, "spoof");
    for (const invalid of [NaN, Infinity, -1, 1.5, "0"]) send(invalid);
    await act(() => vi.advanceTimersByTime(200));
    expect(reveal).not.toHaveBeenCalled();
    send(0);
    send(1);
    send(2);
    await act(() => vi.advanceTimersByTime(200));
    expect(reveal).toHaveBeenCalledExactlyOnceWith(2);
    send(3);
    rerender(view("<p>new source</p>"));
    send(4);
    await act(() => vi.advanceTimersByTime(200));
    expect(reveal).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("does not let an out-of-range index overwrite a pending reveal", async () => {
    const reveal = vi.fn();
    const view = () => (
      <ArtifactPolicyProvider
        value={{ ...policy, revealAnchorCount: 1, onRevealAnchor: reveal }}
      >
        <Markdown>{markdown("<p>one</p>")}</Markdown>
      </ArtifactPolicyProvider>
    );
    render(view());
    const frame = await screen.findByTitle<HTMLIFrameElement>("Tour visual");
    const channel = /var channel = "([^"]+)"/.exec(frame.srcdoc)?.[1];
    const send = (index: number) =>
      fireEvent(
        window,
        new MessageEvent("message", {
          source: frame.contentWindow,
          data: { type: "refact-review-reveal-anchor", channel, index },
        }),
      );

    vi.useFakeTimers();
    send(0);
    send(99);
    await act(() => vi.advanceTimersByTime(200));
    expect(reveal).toHaveBeenCalledExactlyOnceWith(0);

    reveal.mockClear();
    send(99);
    await act(() => vi.advanceTimersByTime(200));
    expect(reveal).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("keeps the iframe and generation on callback changes but rebinds on revision changes", async () => {
    const first = vi.fn();
    const second = vi.fn();
    const view = (
      bindingKey: string,
      onRevealAnchor: (index: number) => void,
    ) => (
      <ArtifactPolicyProvider value={{ ...policy, bindingKey, onRevealAnchor }}>
        <Markdown>{markdown("<button>counter</button>")}</Markdown>
      </ArtifactPolicyProvider>
    );
    const { rerender } = render(view("review:step:candidate:1", first));
    const frame = await screen.findByTitle<HTMLIFrameElement>("Tour visual");
    const sourceWindow = frame.contentWindow;
    const source = frame.srcdoc;
    const channel = /var channel = "([^"]+)"/.exec(source)?.[1];
    const send = () =>
      fireEvent(
        window,
        new MessageEvent("message", {
          source: sourceWindow,
          data: { type: "refact-review-reveal-anchor", channel, index: 0 },
        }),
      );
    vi.useFakeTimers();
    send();
    rerender(view("review:step:candidate:1", second));
    expect(screen.getByTitle("Tour visual")).toBe(frame);
    expect(frame.contentWindow).toBe(sourceWindow);
    expect(frame.srcdoc).toBe(source);
    await act(() => vi.advanceTimersByTime(200));
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledExactlyOnceWith(0);
    send();
    rerender(view("review:step:candidate:2", second));
    const replacement = screen.getByTitle<HTMLIFrameElement>("Tour visual");
    expect(replacement).not.toBe(frame);
    expect(replacement.srcdoc).not.toBe(source);
    send();
    fireEvent(
      window,
      new MessageEvent("message", {
        source: replacement.contentWindow,
        data: { type: "refact-review-reveal-anchor", channel, index: 0 },
      }),
    );
    await act(() => vi.advanceTimersByTime(200));
    expect(second).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });

  it("lets a tour page flow at its full height while an embedded visual stays bounded", async () => {
    render(
      <ArtifactPolicyProvider
        value={{
          mode: "review-seamless",
          page: { previews: [], onAction: () => undefined },
        }}
      >
        <ArtifactBlock code="<p>page</p>" />
      </ArtifactPolicyProvider>,
    );
    const frame = await screen.findByTitle<HTMLIFrameElement>("Tour visual");
    const channel = /var channel="([^"]+)"/.exec(frame.srcdoc)?.[1];
    expect(channel).toBeTruthy();
    vi.useFakeTimers();
    fireEvent(
      window,
      new MessageEvent("message", {
        source: frame.contentWindow,
        data: { type: "refact-artifact-resize", height: 5000, channel },
      }),
    );
    await act(() => vi.advanceTimersByTime(50));
    expect(frame.style.height).toBe("5000px");
    vi.useRealTimers();
  });

  it("bounds finite heights and ignores invalid or unbound resize messages", async () => {
    render(
      <ArtifactPolicyProvider value={policy}>
        <Markdown>{markdown("<p>height</p>")}</Markdown>
      </ArtifactPolicyProvider>,
    );
    const frame = await screen.findByTitle<HTMLIFrameElement>("Tour visual");
    const channel = /var channel = "([^"]+)"/.exec(frame.srcdoc)?.[1];
    const resize = (height: unknown, key = channel) =>
      fireEvent(
        window,
        new MessageEvent("message", {
          source: frame.contentWindow,
          data: { type: "refact-artifact-resize", height, channel: key },
        }),
      );
    const original = frame.style.height;
    vi.useFakeTimers();
    resize(Infinity);
    resize(NaN);
    resize(-1);
    resize(700, "wrong");
    await act(() => vi.advanceTimersByTime(50));
    expect(frame.style.height).toBe(original);
    resize(100000);
    await act(() => vi.advanceTimersByTime(50));
    expect(frame.style.height).toBe("800px");
    vi.useRealTimers();
  });

  it("only sends allowlisted resolved theme values and updates without replacing source", async () => {
    const tokens = {
      "--rf-color-fg": "resolved-color",
      "--secret": "private-token-value",
      "--rf-font-ui": "url(https://example.test)",
    };
    expect(
      reviewThemePayload({ appearance: "dark", reducedMotion: true, tokens })
        .tokens,
    ).toEqual({ "--rf-color-fg": "resolved-color" });
    const view = (appearance: "dark" | "light", reducedMotion: boolean) => (
      <ArtifactPolicyProvider
        value={{ ...policy, theme: { appearance, reducedMotion, tokens } }}
      >
        <Markdown>{markdown("<p>theme</p>")}</Markdown>
      </ArtifactPolicyProvider>
    );
    const { rerender } = render(view("dark", false));
    const frame = await screen.findByTitle<HTMLIFrameElement>("Tour visual");
    const target = frame.contentWindow;
    if (!target) throw new Error("Missing iframe window");
    const post = vi.spyOn(target, "postMessage");
    const source = frame.srcdoc;
    fireEvent.load(frame);
    expect(post).toHaveBeenLastCalledWith(
      expect.objectContaining({
        type: "refact-review-theme",
        theme: {
          appearance: "dark",
          reducedMotion: false,
          tokens: { "--rf-color-fg": "resolved-color" },
        },
      }),
      "*",
    );
    rerender(view("light", true));
    expect(post).toHaveBeenLastCalledWith(
      expect.objectContaining({
        theme: {
          appearance: "light",
          reducedMotion: true,
          tokens: { "--rf-color-fg": "resolved-color" },
        },
      }),
      "*",
    );
    expect(frame.srcdoc).toBe(source);
    expect(source).not.toContain("window.refactReview =");
    expect(source).not.toContain("private-token-value");
  });

  it("preserves automatic chat preview", async () => {
    const { container } = render(
      <Markdown>{markdown("<p>chat</p>")}</Markdown>,
    );
    await screen.findByTitle("HTML Preview");
    expect(
      container.querySelector("iframe")?.getAttribute("srcdoc"),
    ).not.toContain("connect-src 'none'");
  });
});
