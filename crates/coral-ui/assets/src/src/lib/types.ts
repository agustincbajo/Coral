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
