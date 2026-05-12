// NOTE(coral-ui frontend): mirrors the OpenAPI-ish contract documented
// in docs/PRD-v0.32-webui.md. Field naming follows the Rust crate's
// snake_case serde output verbatim.

export type PageType =
  | "module"
  | "concept"
  | "entity"
  | "flow"
  | "decision"
  | "synthesis"
  | "operation"
  | "source"
  | "gap"
  | "index"
  | "log"
  | "schema"
  | "readme"
  | "reference"
  | "interface";

export const ALL_PAGE_TYPES: PageType[] = [
  "module",
  "concept",
  "entity",
  "flow",
  "decision",
  "synthesis",
  "operation",
  "source",
  "gap",
  "index",
  "log",
  "schema",
  "readme",
  "reference",
  "interface",
];

export type Status =
  | "draft"
  | "reviewed"
  | "verified"
  | "stale"
  | "archived"
  | "reference";

export const ALL_STATUSES: Status[] = [
  "draft",
  "reviewed",
  "verified",
  "stale",
  "archived",
  "reference",
];

export interface PageSummary {
  slug: string;
  page_type: PageType;
  status: Status;
  confidence: number;
  generated_at: string;
  valid_from: string | null;
  valid_to: string | null;
  backlinks_count: number;
  sources_count: number;
  path: string;
}

export interface PagesMeta {
  total: number;
  limit: number;
  offset: number;
  next_offset: number | null;
}

export interface PagesEnvelope {
  data: PageSummary[];
  meta: PagesMeta;
}

export interface Frontmatter {
  slug?: string;
  page_type?: PageType;
  status?: Status;
  confidence?: number;
  generated_at?: string;
  valid_from?: string | null;
  valid_to?: string | null;
  sources?: string[];
  tags?: string[];
  // NOTE(coral-ui frontend): frontmatter is open-shape; we let unknown
  // fields fall through.
  [key: string]: unknown;
}

export interface PageDetail {
  frontmatter: Frontmatter;
  body: string;
  backlinks: string[];
}

export interface SearchHit {
  slug: string;
  score: number;
  snippet: string;
}

export interface GraphNode {
  id: string;
  label: string;
  page_type: PageType;
  status: Status;
  confidence: number;
  degree: number;
  valid_from: string | null;
  valid_to: string | null;
}

export interface GraphEdge {
  source: string;
  target: string;
}

export interface GraphPayload {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface Stats {
  page_count: number;
  status_breakdown: Record<string, number>;
  page_type_breakdown: Record<string, number>;
  avg_confidence: number;
  total_backlinks: number;
}

export interface ErrorEnvelope {
  error: {
    code: string;
    message: string;
    hint?: string;
  };
}

// -- M2/M3 surfaces ---------------------------------------------------

export interface InterfaceSummary {
  slug: string;
  repo: string;
  status: Status;
  confidence: number;
  sources: string[];
  valid_from: string | null;
  valid_to: string | null;
  backlinks_count: number;
}

export type DriftSeverity = "critical" | "high" | "medium" | "low" | "info";

export interface DriftFinding {
  severity?: DriftSeverity | string;
  message?: string;
  // NOTE(coral-ui frontend): contract reports are open-shape from the
  // backend — we surface known fields and tolerate extras.
  [key: string]: unknown;
}

export interface DriftReport {
  slug?: string;
  repo?: string;
  status?: string;
  findings?: DriftFinding[];
  generated_at?: string;
  [key: string]: unknown;
}

export interface AffectedMeta {
  total: number;
  since: string;
}

export interface AffectedEnvelope {
  data: string[];
  meta: AffectedMeta;
}

export type GuaranteeVerdict = "GREEN" | "YELLOW" | "RED";

export interface GuaranteeCheck {
  name: string;
  passed: number;
  warnings: number;
  failures: number;
  detail?: string;
}

export interface GuaranteeResult {
  verdict: GuaranteeVerdict;
  checks: GuaranteeCheck[];
}

export interface GuaranteeEnvelope {
  data: GuaranteeResult;
  meta: { exit_code: number };
}

export interface ToolRunResult {
  status: "ok" | "error" | string;
  exit_code: number;
  stdout_tail: string;
  stderr_tail: string;
  duration_ms: number;
}

export interface VerifyToolInput {
  env?: string;
}

export interface RunTestToolInput {
  services?: string[];
  kinds?: string[];
  tags?: string[];
  env?: string;
}

export interface UpToolInput {
  env?: string;
}

export interface DownToolInput {
  env?: string;
  volumes?: boolean;
}
