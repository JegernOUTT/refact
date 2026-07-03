export const BUG_REPORT_REPO = "JegernOUTT/refact";
export const GITHUB_NEW_ISSUE_URL = `https://github.com/${BUG_REPORT_REPO}/issues/new`;
export const GITHUB_BODY_CHAR_LIMIT = 6000;
export const GITHUB_TITLE_CHAR_LIMIT = 256;

const TRUNCATION_NOTICE =
  "\n\n…(truncated — full logs are in the attached bundle)";

export type GithubIssueDraft = {
  title: string;
  labels: string[];
  body: string;
};

export function truncateIssueBody(body: string): string {
  if (body.length <= GITHUB_BODY_CHAR_LIMIT) return body;
  return (
    body.slice(0, GITHUB_BODY_CHAR_LIMIT - TRUNCATION_NOTICE.length) +
    TRUNCATION_NOTICE
  );
}

export function buildGithubIssueUrl(draft: GithubIssueDraft): string {
  const url = new URL(GITHUB_NEW_ISSUE_URL);
  const title = draft.title.trim().slice(0, GITHUB_TITLE_CHAR_LIMIT);
  if (title) {
    url.searchParams.set("title", title);
  }
  const labels = draft.labels.map((label) => label.trim()).filter(Boolean);
  if (labels.length > 0) {
    url.searchParams.set("labels", labels.join(","));
  }
  const body = truncateIssueBody(draft.body);
  if (body.trim()) {
    url.searchParams.set("body", body);
  }
  return url.toString();
}
