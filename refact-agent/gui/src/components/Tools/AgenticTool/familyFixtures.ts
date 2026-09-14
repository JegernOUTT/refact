import {
  buildAgenticToolViewModel,
  type AgenticToolViewModel,
} from "./agenticToolModel";

export function familyModel(input: {
  toolName: string;
  args?: Record<string, unknown>;
  argsRaw?: string;
  content?: string | null;
  hasResult?: boolean;
  toolFailed?: boolean;
  isBusy?: boolean;
}): AgenticToolViewModel {
  const model = buildAgenticToolViewModel({
    toolName: input.toolName,
    argumentsRaw:
      input.argsRaw ?? (input.args ? JSON.stringify(input.args) : "{}"),
    content: input.content ?? null,
    hasResult: input.hasResult ?? true,
    toolFailed: input.toolFailed,
    isBusy: input.isBusy,
    historical: input.hasResult !== false,
  });
  if (!model) throw new Error(`${input.toolName} is not an agentic tool`);
  return model;
}

export function markdownRows(count: number): string {
  const rows = Array.from(
    { length: count },
    (_value, index) => `| T-${index + 1} | Card ${index + 1} | P1 |`,
  );
  return [
    "# Ready Cards",
    "",
    "| Card | Title | Priority |",
    "|------|-------|----------|",
    ...rows,
  ].join("\n");
}
