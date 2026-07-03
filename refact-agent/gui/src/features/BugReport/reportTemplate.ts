import type { BugReportContext } from "../../services/refact/bugReport";

export const MAX_TEMPLATE_ERRORS = 4;
const MAX_ERROR_CHARS = 220;

export type TemplateErrorEntry = {
  source: string;
  message: string;
};

export function buildReportTemplate(args: {
  context?: BugReportContext;
  errors: TemplateErrorEntry[];
  host: string;
}): string {
  const errorLines = args.errors
    .slice(0, MAX_TEMPLATE_ERRORS)
    .map(
      (entry) =>
        `- [${entry.source}] ${entry.message.slice(0, MAX_ERROR_CHARS)}`,
    );

  const envLines: string[] = [];
  if (args.context) {
    envLines.push(
      `- Engine: refact-lsp v${args.context.engine_version} · http :${args.context.http_port}`,
    );
    envLines.push(`- OS: ${args.context.os}`);
  }
  envLines.push(`- GUI host: ${args.host}`);

  const sections: string[] = [
    "### What happened",
    "",
    "### Expected behavior",
    "",
    "### Steps to reproduce",
    "1. ",
    "",
  ];
  if (errorLines.length > 0) {
    sections.push("### Recent errors (auto-attached)", ...errorLines, "");
  }
  sections.push("### Environment (auto-filled)", ...envLines);
  return sections.join("\n");
}
