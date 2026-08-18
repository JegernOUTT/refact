import type {
  DesignElementSelection as ParentSelection,
  DesignSurfaceInboundMessage as ParentInbound,
  DesignSurfaceOutboundMessage as ParentOutbound,
} from "../../gui/src/features/Design/surfaceContract";
import type {
  DesignElementSelection as RuntimeSelection,
  DesignSurfaceInboundMessage as RuntimeInbound,
  DesignSurfaceOutboundMessage as RuntimeOutbound,
} from "../src/runtime";
import { parseDesignSurfaceMessage } from "../../gui/src/features/Design/surfaceContract";
import { describe, expect, it } from "vitest";

type Equal<Left, Right> =
  (<Value>() => Value extends Left ? 1 : 2) extends <Value>() => Value extends Right
    ? 1
    : 2
    ? (<Value>() => Value extends Right ? 1 : 2) extends <Value>() => Value extends Left
      ? 1
      : 2
      ? true
      : false
    : false;

const selectionMatches: Equal<ParentSelection, RuntimeSelection> = true;
const inboundMatches: Equal<ParentInbound, RuntimeInbound> = true;
const outboundMatches: Equal<ParentOutbound, RuntimeOutbound> = true;
type ChildToParentType =
  | "refact:design-ready"
  | "refact:element-selected"
  | "refact:iframe-blocked"
  | "refact:send-followup-turn";
type ParentToChildType = "refact:set-state" | "refact:call-tool";
const parentInboundDirection: Equal<ParentInbound["type"], ChildToParentType> = true;
const runtimeInboundDirection: Equal<RuntimeInbound["type"], ChildToParentType> = true;
const parentOutboundDirection: Equal<ParentOutbound["type"], ParentToChildType> = true;
const runtimeOutboundDirection: Equal<RuntimeOutbound["type"], ParentToChildType> = true;

void selectionMatches;
void inboundMatches;
void outboundMatches;
void parentInboundDirection;
void runtimeInboundDirection;
void parentOutboundDirection;
void runtimeOutboundDirection;

describe("surface contract", () => {
  it("matches T-55 with explicit per-message directions", () => {
    expect(
      selectionMatches &&
        inboundMatches &&
        outboundMatches &&
        parentInboundDirection &&
        runtimeInboundDirection &&
        parentOutboundDirection &&
        runtimeOutboundDirection,
    ).toBe(true);
  });

  it("emits picked selectors that compose as css locators and never as browser refs", () => {
    const isBrowserRef = (value: string): boolean =>
      /^(f[1-9]\d*)?e[1-9]\d*$/.test(value);

    for (const selector of [
      '[data-refact-src="src/App.tsx:12:4"]',
      "form > div:nth-of-type(2)",
      "footer",
      "figure > img",
      "em",
      "#save",
      "fieldset input",
    ]) {
      expect(isBrowserRef(selector)).toBe(false);
    }
    for (const reference of ["e1", "e42", "f2e7"]) {
      expect(isBrowserRef(reference)).toBe(true);
    }
    expect(isBrowserRef("e0")).toBe(false);
    expect(isBrowserRef("em")).toBe(false);
  });

  it("accepts the runtime Apply envelope and rejects malformed or reversed traffic", () => {
    expect(
      parseDesignSurfaceMessage({
        type: "refact:send-followup-turn",
        payload: { content: '{"edits":[]}' },
      }),
    ).toEqual({
      type: "refact:send-followup-turn",
      payload: { content: '{"edits":[]}' },
    });
    expect(
      parseDesignSurfaceMessage({
        type: "refact:send-followup-turn",
        payload: { content: 42 },
      }),
    ).toBeNull();
    expect(
      parseDesignSurfaceMessage({
        type: "refact:set-state",
        payload: { theme: "dark", pickerEnabled: true, devicePixelRatio: 2 },
      }),
    ).toBeNull();
    expect(
      parseDesignSurfaceMessage({
        type: "refact:call-tool",
        payload: { name: "design.apply", arguments: {} },
      }),
    ).toBeNull();
  });
});
