const SKILL_REPORT_PREFIX = "## Skill Report:";

export function isSkillReportContent(content: string): boolean {
  return content.startsWith(SKILL_REPORT_PREFIX);
}

export function parseSkillReport(content: string): {
  skillName: string;
  report: string;
} | null {
  if (!content.startsWith(SKILL_REPORT_PREFIX)) return null;
  const firstNewline = content.indexOf("\n");
  if (firstNewline === -1)
    return {
      skillName: content.slice(SKILL_REPORT_PREFIX.length).trim(),
      report: "",
    };
  const skillName = content
    .slice(SKILL_REPORT_PREFIX.length, firstNewline)
    .trim();
  const report = content.slice(firstNewline + 1).trim();
  return { skillName, report };
}
