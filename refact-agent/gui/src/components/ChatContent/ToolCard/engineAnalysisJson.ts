export type AnalysisSeverity = "Low" | "Medium" | "High" | "Critical";

export interface AnalysisMetric {
  key: string;
  value: string;
}
export interface AnalysisRow {
  raw: string;
  line: number;
  lead: string | null;
  severity: AnalysisSeverity | null;
  severityLabel: string | null;
  title: string;
  detail: string | null;
  metrics: AnalysisMetric[];
  paths: string[];
  tags: string[];
}
export interface AnalysisSection {
  title: string;
  line: number;
  titleIsPath: boolean;
  rows: AnalysisRow[];
  metrics: AnalysisMetric[] | null;
}
export interface AnalysisReport {
  warnings: string[];
  headline: string | null;
  indexState: AnalysisMetric[];
  indexStateRaw: string | null;
  facts: AnalysisMetric[];
  sections: AnalysisSection[];
  pathPrefix: string | null;
  isEmpty: boolean;
}

interface BaseResult {
  tool: string;
  summary: string;
}
export interface UiProbeResult extends BaseResult {
  matrix: Array<Record<string, unknown>>;
  target_count: number;
  viewport_count: number;
  theme_count: number;
  state_count: number;
}
export interface MarkElementsResult extends BaseResult {
  marks: Array<Record<string, unknown>>;
  artifact: Record<string, unknown>;
}
export interface ContrastAuditResult extends BaseResult {
  findings: Array<Record<string, unknown>>;
  raw_colors: Array<Record<string, unknown>>;
  thresholds: Record<string, number>;
}
export interface ImageRegionResult extends BaseResult {
  source: string;
  region: Record<string, number>;
  artifact: Record<string, unknown>;
}
export interface VisualDiffResult extends BaseResult {
  baseline: string;
  baseline_updated: boolean;
  changed_pixels: number;
  changed_percent: number;
  regions: Array<Record<string, unknown>>;
  artifact: Record<string, unknown>;
}
export interface DesignSystemResult extends BaseResult {
  detected: boolean;
  scope: string;
  looked_for: string[];
  scanned_files: number;
  scanned_bytes: number;
  scan_truncated: boolean;
  token_sources: string[];
  detected_prefixes: string[];
  token_count: number;
  token_output_count: number;
  tokens_truncated: boolean;
  token_categories: Record<string, number>;
  tokens: Record<string, unknown>;
  component_inventory_source: string;
  component_count: number;
  components_truncated: boolean;
  components: Array<Record<string, unknown>>;
  drift_count: number;
  findings_truncated: boolean;
  drift: Array<Record<string, unknown>>;
}
interface IndexState {
  queued: number;
  cross_file_edges: number;
  cross_file_ready: boolean;
}
interface RankedSymbol {
  symbol: string;
  path: string;
  score: number;
}
interface RankedFile {
  path: string;
  score: number;
}
export interface CodegraphOverviewResult extends BaseResult {
  counts: { nodes: number; edges: number; files: number };
  index_state: IndexState;
  scc_count: number;
  largest_scc: number;
  component_count: number;
  top_pagerank: RankedSymbol[];
  top_betweenness: RankedSymbol[];
  file_centrality: {
    top_pagerank: RankedFile[];
    top_betweenness: RankedFile[];
  };
  community_count: number;
  dead_code_count: number;
  partial: boolean;
  warning?: string;
  communities: { label: string; member_count: number; cohesion: number }[];
  execution_flows: { entry: string; reaches: number; depth: number }[];
  dead_code: {
    name: string;
    path: string;
    reason: string;
    confidence: number;
  }[];
  entry_points: string[];
  api_contract_files: string[];
}
interface HealthFunction {
  name: string;
  line1: number;
  complexity: number;
  nesting: number;
  loc: number;
  maintainability: number;
  maintainability_index: number;
}
interface HealthFile {
  path: string;
  lang: string;
  score: number;
  grade: string;
  complexity: number;
  maintainability: number;
  maintainability_index: number;
  maintainability_signal: number;
  max_complexity: number;
  avg_maintainability: number;
  function_count: number;
  duplication_pct: number;
  dry_violation: boolean;
  defect_score: number;
  maintainability_score: number;
  performance_score: number;
  biomarker_count: number;
  refactoring_count: number;
  functions: HealthFunction[];
  findings: HealthBiomarker[];
  health_impact: HealthBiomarker[];
  cache_hit: boolean;
  refactorings: HealthRefactoring[];
}
interface HealthBiomarker {
  biomarker: string;
  category: string;
  dimension: string;
  severity: AnalysisSeverity;
  line: number;
  detail: string;
  deduction?: number;
  hot_path?: boolean;
  capped?: boolean;
}
interface HealthRefactoring {
  kind: string;
  target: string;
  line: number;
  rationale: string;
  impact: number;
  effort: string;
}
export interface CodeHealthResult extends BaseResult {
  index_state: Record<string, unknown>;
  file_category?: string;
  file_role?: string;
  aggregate: {
    file_count: number;
    function_count: number;
    avg_score: number;
    grade: string;
    max_complexity: number;
    avg_maintainability: number;
    avg_maintainability_index: number;
    avg_maintainability_signal: number;
    avg_duplication_pct: number;
    biomarker_count: number;
    refactoring_count: number;
  };
  files: HealthFile[];
  call_graph: { caller: string; callee: string }[];
  coverage?: {
    label: string;
    line_pct: number;
    branch_pct: number;
    files_below_50: number;
  };
  warm_cache: boolean;
}
export interface AnalysisFinding {
  path: string;
  biomarker: string;
  category: string;
  dimension: string;
  severity: AnalysisSeverity;
  line: number;
  detail: string;
}
export interface GitRiskResult extends BaseResult {
  commits_analyzed: number;
  agent_authored_pct: number;
  hotspots: {
    path: string;
    churn: number;
    risk: number;
    churn_risk: number;
    churn_percentile: number;
    temporal_score: number;
    change_entropy: number;
    change_entropy_pct: number;
    bus_factor: number;
    ownership_risk: boolean;
    knowledge_loss: boolean;
  }[];
  ownership: {
    path: string;
    top_owner: string;
    top_owner_share: number;
    bus_factor: number;
    owner_count: number;
    ownership_risk: boolean;
    knowledge_loss: boolean;
    owners: { author: string; commits: number; share: number }[];
  }[];
  co_change: { path_a: string; path_b: string; count: number }[];
  coupling: { a: string; b: string; strength: number; co_changes: number }[];
  reviewers: { author: string; score: number }[];
  findings: AnalysisFinding[];
  recent_commit_risks: {
    sha: string;
    summary: string;
    risk: number;
    top_factor_names: string[];
  }[];
}
export interface CodeDuplicationResult extends BaseResult {
  aggregate: {
    file_count: number;
    clone_pair_count: number;
    duplication_pct: number;
    duplication_percent: number;
  };
  clones: {
    path_a: string;
    path_b: string;
    line_a: number;
    line_b: number;
    a_start_line: number;
    a_end_line: number;
    b_start_line: number;
    b_end_line: number;
    lines: number;
    token_len: number;
    co_change: number;
  }[];
  dry_violations: AnalysisFinding[];
  test_smells: AnalysisFinding[];
}
interface BlastImpact {
  path: string;
  symbol: string;
  distance: number;
  via: string;
  kind: "behavioral" | "structural";
}
export interface PrBlastResult extends BaseResult {
  max_depth: number;
  changed_files: string[];
  directly_impacted: BlastImpact[];
  transitively_impacted: BlastImpact[];
  impacted_file_count: number;
  risk_score: number;
  suggested_reviewers: { author: string; score: number }[];
  index_state: Record<string, unknown>;
  partial: boolean;
  warning?: string;
}
export interface DeadCodeResult extends BaseResult {
  entries: {
    name: string;
    path: string;
    line: number;
    reason: string;
    confidence: number;
    git_recency: number;
    incoming_edges: number;
  }[];
  shown: number;
  total_candidates: number;
  index_state: {
    queued: number;
    dirty_paths: number;
    pending_refs: number;
    cross_file_edges: number;
    cross_file_ready: boolean;
  };
  partial: boolean;
  warning?: string;
}
export interface SecurityScanResult extends BaseResult {
  path: string;
  lang: string;
  finding_count: number;
  counts: Record<string, number>;
  findings: {
    rule: string;
    severity: AnalysisSeverity;
    line: number;
    snippet: string;
  }[];
  omitted: number;
}
export interface CodeMapResult extends BaseResult {
  files_count: number;
  page_count: number;
  link_count: number;
  query?: string;
  index_state?: Record<string, unknown>;
  partial: boolean;
  warning?: string;
  top_files: RankedFile[];
  backlink_hubs: { path: string; count: number }[];
  pages: {
    title: string;
    kind: string;
    score: number;
    paths: string[];
    signals: string[];
    symbols: Record<string, unknown>;
    visibility: Record<string, unknown>;
    links: { target_path: string; labels: string[]; count: number }[];
    content: string;
  }[];
  markdown?: string;
}
export interface CodeWhyResult extends BaseResult {
  query: string;
  source_count: number;
  commits_analyzed: number;
  decisions: {
    kind: string;
    confidence: number;
    corroboration: number;
    source_kind: string;
    source_ref: string;
    summary: string;
    provenance_tags: string[];
  }[];
  related: { from: string; relation: string; to: string }[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
function str(record: Record<string, unknown>, key: string): string | null {
  const value = record[key];
  return typeof value === "string" ? value : null;
}
function num(record: Record<string, unknown>, key: string): number | null {
  const value = record[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}
function bool(record: Record<string, unknown>, key: string): boolean | null {
  const value = record[key];
  return typeof value === "boolean" ? value : null;
}
function rec(
  record: Record<string, unknown>,
  key: string,
): Record<string, unknown> | null {
  const value = record[key];
  return isRecord(value) ? value : null;
}
function arr(record: Record<string, unknown>, key: string): unknown[] | null {
  const value = record[key];
  return Array.isArray(value) ? value : null;
}
function strings(
  record: Record<string, unknown>,
  key: string,
): string[] | null {
  const values = arr(record, key);
  return values?.every((value): value is string => typeof value === "string")
    ? values
    : null;
}
function severity(record: Record<string, unknown>): AnalysisSeverity | null {
  const value = str(record, "severity");
  return value === "Low" ||
    value === "Medium" ||
    value === "High" ||
    value === "Critical"
    ? value
    : null;
}
const metric = (
  key: string,
  value: string | number | boolean,
): AnalysisMetric => ({ key, value: String(value) });
function objectMetrics(
  value: Record<string, unknown> | null,
): AnalysisMetric[] {
  if (!value) return [];
  return Object.entries(value).flatMap(([key, item]) =>
    typeof item === "string" ||
    typeof item === "number" ||
    typeof item === "boolean"
      ? [metric(key, item)]
      : [],
  );
}

export function parseEngineAnalysisJson(
  content: string,
): Record<string, unknown> | null {
  try {
    const value: unknown = JSON.parse(content);
    return isRecord(value) ? value : null;
  } catch {
    return null;
  }
}

function commonPathPrefix(paths: string[]): string | null {
  const split = paths
    .filter((path) => path.startsWith("/"))
    .map((path) => path.split("/"));
  if (split.length === 0) return null;
  const first = split[0];
  let shared = 0;
  for (let index = 0; index < first.length - 1; index++) {
    if (split.every((segments) => segments[index] === first[index])) shared++;
    else break;
  }
  return shared < 3 ? null : `${first.slice(0, shared).join("/")}/`;
}
export function shortenPath(path: string, prefix: string | null): string {
  return prefix && path.startsWith(prefix) ? path.slice(prefix.length) : path;
}

type RowInput = {
  title: string;
  detail?: string | null;
  lead?: string | null;
  severity?: AnalysisSeverity | null;
  severityLabel?: string | null;
  metrics?: AnalysisMetric[];
  paths?: string[];
  tags?: string[];
};
type SectionInput = { title: string; rows: RowInput[]; titleIsPath?: boolean };
type Built = {
  facts: AnalysisMetric[];
  index: AnalysisMetric[];
  sections: SectionInput[];
};
function mapRows(
  values: unknown[] | null,
  mapper: (value: Record<string, unknown>) => RowInput | null,
): RowInput[] | null {
  if (!values) return null;
  const rows: RowInput[] = [];
  for (const value of values) {
    if (!isRecord(value)) return null;
    const row = mapper(value);
    if (!row) return null;
    rows.push(row);
  }
  return rows;
}
function biomarkerRow(
  value: Record<string, unknown>,
  path: string | null,
): RowInput | null {
  const title = str(value, "biomarker");
  const detail = str(value, "detail");
  const tone = severity(value);
  const line = num(value, "line");
  if (title === null || detail === null || tone === null || line === null)
    return null;
  const category = str(value, "category");
  const dimension = str(value, "dimension");
  const deduction = num(value, "deduction");
  const metrics = [metric("line", line)];
  if (deduction !== null) metrics.push(metric("deduction", deduction));
  const tags = [category, dimension].filter(
    (item): item is string => item !== null,
  );
  if (bool(value, "hot_path") === true) tags.push("hot-path");
  if (bool(value, "capped") === true) tags.push("capped");
  return {
    title,
    detail,
    severity: tone,
    severityLabel: tone,
    paths: path === null ? [] : [path],
    metrics,
    tags,
  };
}

function findingRow(value: Record<string, unknown>): RowInput | null {
  const path = str(value, "path");
  const title = str(value, "biomarker");
  const detail = str(value, "detail");
  const tone = severity(value);
  const line = num(value, "line");
  if (
    path === null ||
    title === null ||
    detail === null ||
    tone === null ||
    line === null
  )
    return null;
  const category = str(value, "category");
  const dimension = str(value, "dimension");
  return {
    title,
    detail,
    severity: tone,
    severityLabel: tone,
    paths: [path],
    metrics: [metric("line", line)],
    tags: [category, dimension].filter((item): item is string => item !== null),
  };
}

function overview(value: Record<string, unknown>): Built | null {
  const counts = rec(value, "counts");
  const index = rec(value, "index_state");
  const nodes = counts && num(counts, "nodes");
  const edges = counts && num(counts, "edges");
  const files = counts && num(counts, "files");
  const components = num(value, "component_count");
  const scc = num(value, "scc_count");
  const communities = num(value, "community_count");
  const dead = num(value, "dead_code_count");
  if (
    nodes === null ||
    edges === null ||
    files === null ||
    components === null ||
    scc === null ||
    communities === null ||
    dead === null ||
    !index
  )
    return null;
  const ranked = (key: string) =>
    mapRows(arr(value, key), (item) => {
      const title = str(item, "symbol");
      const path = str(item, "path");
      const score = num(item, "score");
      return title === null || path === null || score === null
        ? null
        : {
            title,
            detail: "Centrality score",
            paths: [path],
            lead: score.toFixed(4),
            metrics: [metric("score", score)],
          };
    });
  const communityRows = mapRows(arr(value, "communities"), (item) => {
    const title = str(item, "label");
    const members = num(item, "member_count");
    const cohesion = num(item, "cohesion");
    return title === null || members === null || cohesion === null
      ? null
      : {
          title,
          detail: "Detected module community",
          metrics: [metric("members", members), metric("cohesion", cohesion)],
        };
  });
  const flows = mapRows(arr(value, "execution_flows"), (item) => {
    const title = str(item, "entry");
    const reaches = num(item, "reaches");
    const depth = num(item, "depth");
    return title === null || reaches === null || depth === null
      ? null
      : {
          title,
          detail: `Reaches ${reaches} nodes at depth ${depth}`,
          metrics: [metric("reaches", reaches), metric("depth", depth)],
        };
  });
  const deadRows = mapRows(arr(value, "dead_code"), (item) => {
    const title = str(item, "name");
    const path = str(item, "path");
    const detail = str(item, "reason");
    const confidence = num(item, "confidence");
    return title === null ||
      path === null ||
      detail === null ||
      confidence === null
      ? null
      : {
          title,
          detail,
          paths: [path],
          lead: confidence.toFixed(2),
          metrics: [metric("confidence", confidence)],
        };
  });
  const pagerank = ranked("top_pagerank");
  const between = ranked("top_betweenness");
  const fileCentrality = rec(value, "file_centrality");
  const rankedFiles = (key: string) =>
    fileCentrality &&
    mapRows(arr(fileCentrality, key), (item) => {
      const path = str(item, "path");
      const score = num(item, "score");
      return path === null || score === null
        ? null
        : {
            title: path,
            detail: "File centrality score",
            paths: [path],
            lead: score.toFixed(4),
            metrics: [metric("score", score)],
          };
    });
  const filePagerank = rankedFiles("top_pagerank");
  const fileBetween = rankedFiles("top_betweenness");
  const entries = strings(value, "entry_points");
  const api = strings(value, "api_contract_files");
  if (
    !pagerank ||
    !between ||
    !filePagerank ||
    !fileBetween ||
    !communityRows ||
    !flows ||
    !deadRows ||
    !entries ||
    !api ||
    num(value, "largest_scc") === null ||
    typeof value.partial !== "boolean"
  )
    return null;
  const pathRows = (items: string[]): RowInput[] =>
    items.map((path) => ({ title: "", paths: [path] }));
  return {
    facts: [
      metric("Nodes", nodes),
      metric("Edges", edges),
      metric("Files", files),
      metric("Components", components),
      metric("SCC", scc),
      metric("Largest SCC", num(value, "largest_scc") ?? 0),
      metric("Communities", communities),
      metric("Dead code", dead),
    ],
    index: objectMetrics(index),
    sections: [
      { title: "Most central symbols (PageRank)", rows: pagerank },
      { title: "Key connectors (betweenness)", rows: between },
      { title: "Most central files (PageRank)", rows: filePagerank },
      { title: "File connectors (betweenness)", rows: fileBetween },
      { title: "Module communities", rows: communityRows },
      { title: "Execution flows", rows: flows },
      { title: "Likely dead code", rows: deadRows },
      { title: "Likely entry points", rows: pathRows(entries) },
      { title: "API-contract files", rows: pathRows(api) },
    ],
  };
}

function gitRisk(value: Record<string, unknown>): Built | null {
  const commits = num(value, "commits_analyzed");
  const authored = num(value, "agent_authored_pct");
  if (commits === null || authored === null || !Array.isArray(value.ownership))
    return null;
  const hotspots = mapRows(arr(value, "hotspots"), (item) => {
    const path = str(item, "path");
    const churn = num(item, "churn");
    const risk = num(item, "risk");
    const bus = num(item, "bus_factor");
    if (path === null || churn === null || risk === null || bus === null)
      return null;
    const tags = [
      bool(item, "ownership_risk") ? "ownership-risk" : null,
      bool(item, "knowledge_loss") ? "knowledge-loss" : null,
    ].filter((tag): tag is string => tag !== null);
    return {
      title: path,
      detail: "Recency-weighted change hotspot",
      paths: [path],
      metrics: [
        metric("churn", churn),
        metric("risk", risk),
        metric("bus_factor", bus),
      ],
      tags,
    };
  });
  const coChange = mapRows(arr(value, "co_change"), (item) => {
    const a = str(item, "path_a");
    const b = str(item, "path_b");
    const count = num(item, "count");
    return a === null || b === null || count === null
      ? null
      : {
          title: "",
          detail: `${a} and ${b} changed together`,
          paths: [a, b],
          lead: `${count}x`,
          metrics: [metric("count", count)],
        };
  });
  const coupling = mapRows(arr(value, "coupling"), (item) => {
    const a = str(item, "a");
    const b = str(item, "b");
    const strength = num(item, "strength");
    const changes = num(item, "co_changes");
    return a === null || b === null || strength === null || changes === null
      ? null
      : {
          title: "",
          detail: "Files with strong historical coupling",
          paths: [a, b],
          lead: strength.toFixed(2),
          metrics: [
            metric("strength", strength),
            metric("co_changes", changes),
          ],
        };
  });
  const reviewers = mapRows(arr(value, "reviewers"), (item) => {
    const author = str(item, "author");
    const score = num(item, "score");
    return author === null || score === null
      ? null
      : {
          title: author,
          detail: "Ownership-based reviewer",
          lead: score.toFixed(2),
          metrics: [metric("score", score)],
        };
  });
  const findings = mapRows(arr(value, "findings"), findingRow);
  const ownership = mapRows(arr(value, "ownership"), (item) => {
    const path = str(item, "path");
    const owner = str(item, "top_owner");
    const share = num(item, "top_owner_share");
    const bus = num(item, "bus_factor");
    const count = num(item, "owner_count");
    if (
      path === null ||
      owner === null ||
      share === null ||
      bus === null ||
      count === null ||
      !Array.isArray(item.owners)
    )
      return null;
    const tags = [
      bool(item, "ownership_risk") ? "ownership-risk" : null,
      bool(item, "knowledge_loss") ? "knowledge-loss" : null,
    ].filter((tag): tag is string => tag !== null);
    return {
      title: owner,
      detail: `Top owner for ${path}`,
      paths: [path],
      metrics: [
        metric("share", share),
        metric("bus_factor", bus),
        metric("owners", count),
      ],
      tags,
    };
  });
  const recent = mapRows(arr(value, "recent_commit_risks"), (item) => {
    const sha = str(item, "sha");
    const detail = str(item, "summary");
    const risk = num(item, "risk");
    const tags = strings(item, "top_factor_names");
    return sha === null || detail === null || risk === null || !tags
      ? null
      : {
          title: sha,
          detail,
          lead: risk.toFixed(2),
          metrics: [metric("risk", risk)],
          tags,
        };
  });
  if (
    !hotspots ||
    !ownership ||
    !coChange ||
    !coupling ||
    !reviewers ||
    !findings ||
    !recent
  )
    return null;
  return {
    facts: [
      metric("Commits analyzed", commits),
      metric("Agent authored %", Math.round(authored * 1000) / 10),
    ],
    index: [],
    sections: [
      { title: "Hotspots", rows: hotspots },
      { title: "Ownership", rows: ownership },
      { title: "Frequently co-changed", rows: coChange },
      { title: "Strongest coupling", rows: coupling },
      { title: "Suggested reviewers", rows: reviewers },
      { title: "Git-driven biomarkers", rows: findings },
      { title: "Recent commit change-risk", rows: recent },
    ],
  };
}

function duplication(value: Record<string, unknown>): Built | null {
  const aggregate = rec(value, "aggregate");
  if (!aggregate) return null;
  const files = num(aggregate, "file_count");
  const pairs = num(aggregate, "clone_pair_count");
  const pct =
    num(aggregate, "duplication_percent") ??
    (() => {
      const ratio = num(aggregate, "duplication_pct");
      return ratio === null ? null : ratio * 100;
    })();
  const clones = mapRows(arr(value, "clones"), (item) => {
    const a = str(item, "path_a");
    const b = str(item, "path_b");
    const lines = num(item, "lines");
    const tokens = num(item, "token_len");
    const co = num(item, "co_change");
    return a === null ||
      b === null ||
      lines === null ||
      tokens === null ||
      co === null
      ? null
      : {
          title: "",
          detail: `${lines} duplicated lines`,
          paths: [a, b],
          lead: `${tokens} tokens`,
          metrics: [metric("lines", lines), metric("co-change", co)],
        };
  });
  const dry = mapRows(arr(value, "dry_violations"), findingRow);
  const smells = mapRows(arr(value, "test_smells"), findingRow);
  if (
    files === null ||
    pairs === null ||
    pct === null ||
    !clones ||
    !dry ||
    !smells
  )
    return null;
  return {
    facts: [
      metric("Files", files),
      metric("Clone pairs", pairs),
      metric("Duplication %", pct),
    ],
    index: [],
    sections: [
      { title: "Clone pairs", rows: clones },
      { title: "DRY violations", rows: dry },
      { title: "Test smells", rows: smells },
    ],
  };
}

function blast(value: Record<string, unknown>): Built | null {
  const changed = strings(value, "changed_files");
  const impacted = num(value, "impacted_file_count");
  const risk = num(value, "risk_score");
  const index = rec(value, "index_state");
  const impacts = (key: string) =>
    mapRows(arr(value, key), (item) => {
      const title = str(item, "symbol");
      const path = str(item, "path");
      const distance = num(item, "distance");
      const via = str(item, "via");
      const kind = str(item, "kind");
      return title === null ||
        path === null ||
        distance === null ||
        via === null ||
        (kind !== "behavioral" && kind !== "structural")
        ? null
        : {
            title,
            detail: `Reached via ${via} (${kind})`,
            paths: [path],
            lead: `d${distance}`,
            metrics: [metric("distance", distance)],
            tags: [kind],
          };
    });
  const direct = impacts("directly_impacted");
  const transitive = impacts("transitively_impacted");
  const reviewers = mapRows(arr(value, "suggested_reviewers"), (item) => {
    const author = str(item, "author");
    const score = num(item, "score");
    return author === null || score === null
      ? null
      : {
          title: author,
          detail: "Ownership-based reviewer",
          lead: score.toFixed(2),
          metrics: [metric("score", score)],
        };
  });
  if (
    !changed ||
    impacted === null ||
    risk === null ||
    !index ||
    !direct ||
    !transitive ||
    !reviewers ||
    typeof value.partial !== "boolean"
  )
    return null;
  return {
    facts: [
      metric("Changed", changed.length),
      metric("Direct", direct.length),
      metric("Transitive", transitive.length),
      metric("Impacted files", impacted),
      metric("Risk", risk.toFixed(2)),
    ],
    index: objectMetrics(index),
    sections: [
      { title: "Directly impacted symbols", rows: direct },
      { title: "Transitively impacted symbols", rows: transitive },
      { title: "Suggested reviewers", rows: reviewers },
    ],
  };
}

function deadCode(value: Record<string, unknown>): Built | null {
  const shown = num(value, "shown");
  const total = num(value, "total_candidates");
  const index = rec(value, "index_state");
  const entries = arr(value, "entries");
  if (
    shown === null ||
    total === null ||
    !index ||
    !entries ||
    typeof value.partial !== "boolean"
  )
    return null;
  const groups = new Map<string, RowInput[]>();
  for (const raw of entries) {
    if (!isRecord(raw)) return null;
    const name = str(raw, "name");
    const path = str(raw, "path");
    const sourceLine = num(raw, "line");
    const reason = str(raw, "reason");
    const confidence = num(raw, "confidence");
    const recency = num(raw, "git_recency");
    const incoming = num(raw, "incoming_edges");
    if (
      name === null ||
      path === null ||
      sourceLine === null ||
      reason === null ||
      confidence === null ||
      recency === null ||
      incoming === null
    )
      return null;
    const rows = groups.get(path) ?? [];
    rows.push({
      title: name,
      detail: reason,
      lead: confidence.toFixed(2),
      metrics: [
        metric("line", sourceLine),
        metric("confidence", confidence),
        metric("git recency", recency),
        metric("incoming edges", incoming),
      ],
    });
    groups.set(path, rows);
  }
  return {
    facts: [metric("Shown", shown), metric("Matching", total)],
    index: objectMetrics(index),
    sections: Array.from(groups, ([title, rows]) => ({
      title,
      rows,
      titleIsPath: true,
    })),
  };
}

function securityScan(value: Record<string, unknown>): Built | null {
  const path = str(value, "path");
  const lang = str(value, "lang");
  const count = num(value, "finding_count");
  const counts = rec(value, "counts");
  const findings = mapRows(arr(value, "findings"), (item) => {
    const title = str(item, "rule");
    const tone = severity(item);
    const line = num(item, "line");
    const detail = str(item, "snippet");
    return title === null ||
      tone === null ||
      line === null ||
      detail === null ||
      path === null
      ? null
      : {
          title,
          detail,
          severity: tone,
          severityLabel: tone,
          paths: [path],
          metrics: [metric("line", line)],
        };
  });
  if (path === null || lang === null || count === null || !counts || !findings)
    return null;
  return {
    facts: [
      metric("Findings", count),
      metric("Language", lang),
      ...objectMetrics(counts),
    ],
    index: [],
    sections: [{ title: "Findings", rows: findings }],
  };
}

function health(value: Record<string, unknown>): Built | null {
  const aggregate = rec(value, "aggregate");
  const index = rec(value, "index_state");
  const files = arr(value, "files");
  const graph = arr(value, "call_graph");
  if (!aggregate || !index || !files || !graph) return null;
  const fileCount = num(aggregate, "file_count");
  const grade = str(aggregate, "grade");
  const score = num(aggregate, "avg_score");
  if (fileCount === null || grade === null || score === null) return null;
  const sections: SectionInput[] = [];
  for (const raw of files) {
    if (!isRecord(raw)) return null;
    const path = str(raw, "path");
    if (path === null) return null;
    const functions = mapRows(arr(raw, "functions"), (item) => {
      const title = str(item, "name");
      const line = num(item, "line1");
      const complexity = num(item, "complexity");
      const nesting = num(item, "nesting");
      const loc = num(item, "loc");
      const mi = num(item, "maintainability_index");
      return title === null ||
        line === null ||
        complexity === null ||
        nesting === null ||
        loc === null ||
        mi === null
        ? null
        : {
            title,
            detail: `Defined at line ${line}`,
            metrics: [
              metric("complexity", complexity),
              metric("nesting", nesting),
              metric("loc", loc),
              metric("maintainability_index", mi),
            ],
          };
    });
    const findings = mapRows(arr(raw, "findings"), (item) =>
      biomarkerRow(item, path),
    );
    const impact = mapRows(arr(raw, "health_impact"), (item) =>
      biomarkerRow(item, path),
    );
    const refs = mapRows(arr(raw, "refactorings"), (item) => {
      const kind = str(item, "kind");
      const rationale = str(item, "rationale");
      const line = num(item, "line");
      if (kind === null || rationale === null || line === null) return null;
      const metrics = [metric("line", line)];
      const impactScore = num(item, "impact");
      if (impactScore !== null) metrics.push(metric("impact", impactScore));
      const effort = str(item, "effort");
      return {
        title: kind,
        detail: rationale,
        paths: [path],
        metrics,
        tags: effort === null ? [] : [`${effort} effort`],
      };
    });
    if (!functions || !findings || !impact || !refs) return null;
    sections.push({
      title: "Functions",
      rows: functions.map((row) => ({ ...row, paths: [path] })),
    });
    if (impact.length)
      sections.push({ title: "Top health impact contributors", rows: impact });
    if (findings.length) sections.push({ title: "Biomarkers", rows: findings });
    if (refs.length)
      sections.push({ title: "Refactoring targets", rows: refs });
  }
  const calls = mapRows(graph, (item) => {
    const caller = str(item, "caller");
    const callee = str(item, "callee");
    return caller === null || callee === null
      ? null
      : { title: caller, detail: `Calls ${callee}` };
  });
  if (!calls) return null;
  if (calls.length) sections.push({ title: "Call graph", rows: calls });

  const facts = [
    metric("Files", fileCount),
    metric("Grade", grade),
    metric("Score", score),
  ];
  const category = str(value, "file_category");
  if (category !== null) facts.push(metric("Category", category));
  const role = str(value, "file_role");
  if (role !== null) facts.push(metric("Role", role));
  const maxComplexity = num(aggregate, "max_complexity");
  if (maxComplexity !== null)
    facts.push(metric("Max complexity", maxComplexity));
  const mi = num(aggregate, "avg_maintainability_index");
  if (mi !== null) facts.push(metric("Maintainability index", mi));

  const coverage = rec(value, "coverage");
  if (coverage) {
    const linePct = num(coverage, "line_pct");
    const branchPct = num(coverage, "branch_pct");
    const below = num(coverage, "files_below_50");
    if (linePct !== null) facts.push(metric("Coverage lines %", linePct));
    if (branchPct !== null)
      facts.push(metric("Coverage branches %", branchPct));
    if (below !== null) facts.push(metric("Files below 50%", below));
  }

  const index_ = objectMetrics(index);
  if (bool(value, "warm_cache") === true)
    index_.push({ key: "warm cache", value: "hit" });

  return { facts, index: index_, sections };
}

function codeWhy(value: Record<string, unknown>): Built | null {
  const sources = num(value, "source_count");
  const commits = num(value, "commits_analyzed");
  const decisions = mapRows(arr(value, "decisions"), (item) => {
    const title = str(item, "kind");
    const detail = str(item, "summary");
    const confidence = num(item, "confidence");
    const corroboration = num(item, "corroboration");
    const source = str(item, "source_ref");
    const tags = strings(item, "provenance_tags");
    return title === null ||
      detail === null ||
      confidence === null ||
      corroboration === null ||
      source === null ||
      !tags
      ? null
      : {
          title,
          detail,
          lead: confidence.toFixed(2),
          metrics: [
            metric("corroboration", corroboration),
            metric("source", source),
          ],
          tags,
        };
  });
  const related = mapRows(arr(value, "related"), (item) => {
    const title = str(item, "from");
    const relation = str(item, "relation");
    const to = str(item, "to");
    return title === null || relation === null || to === null
      ? null
      : { title, detail: `${relation} ${to}` };
  });
  if (sources === null || commits === null || !decisions || !related)
    return null;
  return {
    facts: [metric("Sources", sources), metric("Commits analyzed", commits)],
    index: [],
    sections: [
      { title: "Decisions", rows: decisions },
      { title: "Related decisions", rows: related },
    ],
  };
}

function codeMap(value: Record<string, unknown>): Built | null {
  const files = num(value, "files_count");
  const pageCount = num(value, "page_count");
  const links = num(value, "link_count");
  const top = mapRows(arr(value, "top_files"), (item) => {
    const path = str(item, "path");
    const score = num(item, "score");
    return path === null || score === null
      ? null
      : {
          title: path,
          detail: "Documentation-worthy file",
          paths: [path],
          lead: score.toFixed(4),
          metrics: [metric("score", score)],
        };
  });
  const hubs = mapRows(arr(value, "backlink_hubs"), (item) => {
    const path = str(item, "path");
    const count = num(item, "count");
    return path === null || count === null
      ? null
      : {
          title: path,
          detail: "Documentation backlink hub",
          paths: [path],
          lead: `${count}x`,
          metrics: [metric("count", count)],
        };
  });
  const pages = mapRows(arr(value, "pages"), (item) => {
    const title = str(item, "title");
    const kind = str(item, "kind");
    const score = num(item, "score");
    const paths = strings(item, "paths");
    const tags = strings(item, "signals");
    const detail = str(item, "content");
    return title === null ||
      kind === null ||
      score === null ||
      !paths ||
      !tags ||
      detail === null
      ? null
      : {
          title,
          detail,
          paths,
          lead: score.toFixed(2),
          metrics: [metric("kind", kind), metric("score", score)],
          tags,
        };
  });
  if (
    files === null ||
    pageCount === null ||
    links === null ||
    !top ||
    !hubs ||
    !pages
  )
    return null;
  return {
    facts: [
      metric("Files", files),
      metric("Pages", pageCount),
      metric("Links", links),
    ],
    index: objectMetrics(rec(value, "index_state")),
    sections: [
      { title: "Documentation-worthy files", rows: top },
      { title: "Backlink hubs", rows: hubs },
      { title: "Pages", rows: pages },
    ],
  };
}

function designTool(
  value: Record<string, unknown>,
  toolName: string,
): Built | null {
  const rows: RowInput[] = [];
  const facts: AnalysisMetric[] = [];
  if (toolName === "ui_probe") {
    const matrix = arr(value, "matrix");
    if (!matrix) return null;
    const targetCount = num(value, "target_count");
    const viewportCount = num(value, "viewport_count");
    const themeCount = num(value, "theme_count");
    const stateCount = num(value, "state_count");
    if (targetCount !== null) facts.push(metric("Targets", targetCount));
    if (viewportCount !== null) facts.push(metric("Viewports", viewportCount));
    if (themeCount !== null) facts.push(metric("Themes", themeCount));
    if (stateCount !== null) facts.push(metric("States", stateCount));
    const matrixRows = mapRows(matrix, (item) => {
      const target = str(item, "target");
      const theme = str(item, "theme");
      const state = str(item, "state");
      const viewport = rec(item, "viewport");
      const width = viewport ? num(viewport, "width") : null;
      const height = viewport ? num(viewport, "height") : null;
      if (target === null || theme === null || state === null) return null;
      return {
        title: target,
        detail: `${theme} · ${state}`,
        lead:
          width !== null && height !== null ? `${width}×${height}` : undefined,
      };
    });
    if (!matrixRows) return null;
    rows.push(...matrixRows);
  } else if (toolName === "mark_elements") {
    const marks = arr(value, "marks");
    if (!marks) return null;
    facts.push(metric("Marks", marks.length));
    const markRows = mapRows(marks, (item) => {
      const markId = num(item, "mark_id");
      const reference = str(item, "ref");
      const role = str(item, "role");
      const name = str(item, "name");
      if (markId === null || reference === null || role === null) return null;
      return {
        title: name ?? role,
        detail: `${role} · ref=${reference}`,
        lead: String(markId),
      };
    });
    if (!markRows) return null;
    rows.push(...markRows);
  } else if (toolName === "contrast_audit") {
    const findings = arr(value, "findings");
    const rawColors = arr(value, "raw_colors");
    if (!findings || !rawColors) return null;
    facts.push(metric("Contrast findings", findings.length));
    facts.push(metric("Non-token colors", rawColors.length));
    const findingRows = mapRows(findings, (item) => {
      const selector = str(item, "selector");
      const ratio = num(item, "ratio");
      const threshold = num(item, "threshold");
      const severity = str(item, "severity");
      if (selector === null || ratio === null || threshold === null)
        return null;
      return {
        title: selector,
        detail: `Required ${threshold.toFixed(1)}:1`,
        lead: `${ratio.toFixed(2)}:1`,
        severity:
          severity === "High" || severity === "Medium" ? severity : undefined,
      };
    });
    const rawColorRows = mapRows(rawColors, (item) => {
      const color = str(item, "color");
      const selector = str(item, "selector");
      return color === null || selector === null
        ? null
        : { title: color, detail: selector, severity: "Low" as const };
    });
    if (!findingRows || !rawColorRows) return null;
    rows.push(...findingRows, ...rawColorRows);
  } else if (toolName === "image_region") {
    const sourceValue = value.source;
    const source =
      typeof sourceValue === "string"
        ? sourceValue
        : isRecord(sourceValue) && typeof sourceValue.display === "string"
          ? sourceValue.display
          : null;
    const region = rec(value, "region");
    if (source === null || !region) return null;
    const width = num(region, "width");
    const height = num(region, "height");
    rows.push({
      title: source,
      detail:
        width !== null && height !== null
          ? `Native crop ${width}×${height}`
          : "Native image crop",
      paths: [source],
    });
  } else if (toolName === "visual_diff") {
    const changedPixels = num(value, "changed_pixels");
    const changedPercent = num(value, "changed_percent");
    const regions = arr(value, "regions");
    if (changedPixels === null || changedPercent === null || !regions)
      return null;
    facts.push(metric("Changed pixels", changedPixels));
    facts.push(metric("Changed %", changedPercent));
    const regionRows = mapRows(regions, (item) => {
      const x = num(item, "x");
      const y = num(item, "y");
      const width = num(item, "width");
      const height = num(item, "height");
      const pixels = num(item, "changed_pixels");
      if ([x, y, width, height, pixels].some((entry) => entry === null))
        return null;
      return {
        title: `Region at ${x},${y}`,
        detail: `${width}×${height}`,
        lead: `${pixels}px`,
        severity: "Medium",
      };
    });
    if (!regionRows) return null;
    rows.push(...regionRows);
  } else {
    return null;
  }
  return { facts, index: [], sections: [{ title: "Results", rows }] };
}

function designSystem(value: Record<string, unknown>): Built | null {
  const detected = bool(value, "detected");
  const scope = str(value, "scope");
  const scannedFiles = num(value, "scanned_files");
  const tokenCount = num(value, "token_count");
  const componentCount = num(value, "component_count");
  const driftCount = num(value, "drift_count");
  const inventorySource = str(value, "component_inventory_source");
  const tokenSources = strings(value, "token_sources");
  const components = mapRows(arr(value, "components"), (item) => {
    const name = str(item, "name");
    const path = str(item, "path");
    const usageCount = num(item, "usage_count");
    const source = str(item, "source");
    const props = strings(item, "props");
    if (
      name === null ||
      path === null ||
      usageCount === null ||
      source === null ||
      props === null
    )
      return null;
    return {
      title: name,
      detail:
        props.length > 0 ? `Props: ${props.join(", ")}` : "No props detected",
      lead: `${usageCount} uses`,
      paths: [path],
      tags: [source],
    };
  });
  const drift = mapRows(arr(value, "drift"), (item) => {
    const kind = str(item, "kind");
    const driftValue = str(item, "value");
    const path = str(item, "path");
    const line = num(item, "line");
    const nearestToken = str(item, "nearest_token");
    const nearestValue = str(item, "nearest_value");
    if (kind === null || driftValue === null || path === null || line === null)
      return null;
    return {
      title: `${path}:${line}`,
      detail:
        nearestToken === null
          ? `Hardcoded ${kind}: ${driftValue}`
          : `Use ${nearestToken}${
              nearestValue === null ? "" : ` (${nearestValue})`
            }`,
      lead: driftValue,
      paths: [path],
      tags: [kind],
      severity: "Medium",
    };
  });
  if (
    detected === null ||
    scope === null ||
    scannedFiles === null ||
    tokenCount === null ||
    componentCount === null ||
    driftCount === null ||
    inventorySource === null ||
    !tokenSources ||
    !components ||
    !drift
  )
    return null;
  const tokenRows: RowInput[] = tokenSources.map((path) => ({
    title: path,
    detail: "Design token source",
    paths: [path],
  }));
  return {
    facts: [
      metric("Detected", detected ? "yes" : "no"),
      metric("Scope", scope),
      metric("Files scanned", scannedFiles),
      metric("Tokens", tokenCount),
      metric("Components", componentCount),
      metric("Drift findings", driftCount),
      metric("Inventory", inventorySource),
    ],
    index: [],
    sections: [
      { title: "Token sources", rows: tokenRows },
      { title: "Components", rows: components },
      { title: "Design drift", rows: drift },
    ],
  };
}

export function buildAnalysisReport(
  toolName: string,
  value: unknown,
): AnalysisReport | null {
  if (!isRecord(value) || str(value, "tool") !== toolName) return null;
  const headline = str(value, "summary");
  if (headline === null) return null;
  const warningValue = value.warning;
  if (warningValue !== undefined && typeof warningValue !== "string")
    return null;
  let built: Built | null = null;
  if (toolName === "codegraph_overview") built = overview(value);
  else if (toolName === "git_risk") built = gitRisk(value);
  else if (toolName === "code_duplication") built = duplication(value);
  else if (toolName === "pr_blast") built = blast(value);
  else if (toolName === "dead_code") built = deadCode(value);
  else if (toolName === "security_scan") built = securityScan(value);
  else if (toolName === "code_health") built = health(value);
  else if (toolName === "code_why") built = codeWhy(value);
  else if (toolName === "code_map") built = codeMap(value);
  else if (toolName === "design_system") built = designSystem(value);
  else if (
    toolName === "ui_probe" ||
    toolName === "mark_elements" ||
    toolName === "contrast_audit" ||
    toolName === "image_region" ||
    toolName === "visual_diff"
  )
    built = designTool(value, toolName);
  if (!built) return null;
  let nextLine = 1;
  const sections = built.sections.map(
    (section): AnalysisSection => ({
      title: section.title,
      line: nextLine++,
      titleIsPath: section.titleIsPath ?? false,
      metrics: null,
      rows: section.rows.map(
        (row): AnalysisRow => ({
          raw: row.title,
          line: nextLine++,
          lead: row.lead ?? null,
          severity: row.severity ?? null,
          severityLabel: row.severityLabel ?? null,
          title: row.title,
          detail: row.detail ?? null,
          metrics: row.metrics ?? [],
          paths: row.paths ?? [],
          tags: row.tags ?? [],
        }),
      ),
    }),
  );
  const allPaths = sections.flatMap((section) => [
    ...(section.titleIsPath ? [section.title] : []),
    ...section.rows.flatMap((row) => row.paths),
  ]);
  return {
    warnings: typeof warningValue === "string" ? [warningValue] : [],
    headline,
    indexState: built.index,
    indexStateRaw: null,
    facts: built.facts,
    sections,
    pathPrefix: commonPathPrefix(allPaths),
    isEmpty: false,
  };
}
