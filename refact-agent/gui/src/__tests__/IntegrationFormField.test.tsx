import { describe, expect, test, vi } from "vitest";
import { render, screen } from "../utils/test-utils";
import { IntegrationFormField } from "../features/Integrations";
import type {
  Integration,
  IntegrationField,
  IntegrationPrimitive,
} from "../services/refact";

const boolField = {
  f_type: "bool",
  f_default: true,
  f_label: "Enable feature",
} as IntegrationField<NonNullable<IntegrationPrimitive>>;

function renderBoolField(values: Integration["integr_values"]) {
  const onChange = vi.fn();
  const view = render(
    <IntegrationFormField
      field={boolField}
      values={values}
      fieldKey="enabled"
      integrationName="test"
      integrationPath="/test.yaml"
      integrationProject="/project"
      onChange={onChange}
    />,
  );

  return { ...view, onChange };
}

describe("IntegrationFormField", () => {
  test("renders persisted boolean false as unchecked", () => {
    renderBoolField({ enabled: false });

    expect(
      screen.getByRole("switch", { name: "Enable feature" }),
    ).not.toBeChecked();
  });

  test("round-trips persisted boolean false as a boolean", async () => {
    const { onChange, user } = renderBoolField({ enabled: false });
    const field = screen.getByRole("switch", { name: "Enable feature" });

    await user.click(field);
    await user.click(field);

    expect(onChange).toHaveBeenLastCalledWith("enabled", false);
  });
});
