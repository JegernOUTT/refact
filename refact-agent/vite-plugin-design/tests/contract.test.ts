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

void selectionMatches;
void inboundMatches;
void outboundMatches;

describe("surface contract", () => {
  it("matches T-55 in both directions", () => {
    expect(selectionMatches && inboundMatches && outboundMatches).toBe(true);
  });
});
