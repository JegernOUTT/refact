import { act as rtlAct, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type {
  IntegrationField,
  IntegrationPrimitive,
} from "../../../services/refact";
import { useFormFields } from "./useFormFields";

type IntegrationFields = Record<
  string,
  IntegrationField<NonNullable<IntegrationPrimitive>>
>;

const fields = {
  url: { f_type: "string" },
  auth_type: { f_type: "string_short", f_extra: true },
} as IntegrationFields;

const act = rtlAct as unknown as (callback: () => void) => void;

describe("useFormFields", () => {
  it("collapses advanced fields when switching integrations", () => {
    const { result, rerender } = renderHook<
      ReturnType<typeof useFormFields>,
      { integrationPath: string; currentFields: IntegrationFields }
    >(
      ({ integrationPath, currentFields }) =>
        useFormFields(currentFields, integrationPath),
      {
        initialProps: {
          integrationPath: "/first.yaml",
          currentFields: fields,
        },
      },
    );

    act(() => {
      result.current.toggleExtraFields();
    });
    expect(result.current.areExtraFieldsRevealed).toBe(true);

    rerender({
      integrationPath: "/first.yaml",
      currentFields: { ...fields },
    });
    expect(result.current.areExtraFieldsRevealed).toBe(true);

    rerender({ integrationPath: "/second.yaml", currentFields: fields });
    expect(result.current.areExtraFieldsRevealed).toBe(false);
  });
});
