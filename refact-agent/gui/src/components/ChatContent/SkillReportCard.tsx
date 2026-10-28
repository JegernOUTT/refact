import React from "react";
import { Zap } from "lucide-react";
import { TranscriptItem } from "./TranscriptItem";
import { ReportActions, ReportMarkdown } from "./ToolCard/ReportToolCard";

interface SkillReportCardProps {
  skillName: string;
  report: string;
  storeKey: string;
}

export const SkillReportCard: React.FC<SkillReportCardProps> = ({
  skillName,
  report,
  storeKey,
}) => (
  <TranscriptItem
    data-testid="skill-report-card"
    actions={report ? <ReportActions markdown={report} /> : null}
    icon={<Zap />}
    meta="report"
    openKey={storeKey}
    output={report ? <ReportMarkdown markdown={report} /> : null}
    sectionLabels={{ output: "Report" }}
    status="done"
    target={skillName}
    verb="Skill"
  />
);

export default SkillReportCard;
