import { getApiBase } from "@/lib/config";
import { useAuthStore } from "@/stores/auth";
import type { ErrorEnvelope } from "@/lib/types";

export class ApiError extends Error {
  code: string;
  hint?: string;
  status: number;

  constructor(code: string, message: string, status: number, hint?: string) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    this.hint = hint;
    this.status = status;
  }
}

interface ApiOptions extends Omit<RequestInit, "body"> {
  body?: unknown;
  // NOTE(coral-ui frontend): when raw=true we don't read .data — the
  // caller wants the full envelope (for endpoints that return meta).
  raw?: boolean;
  // When `auth=false` we skip attaching the Authorization header even
  // if a token is present (used by /health style probes).
  auth?: boolean;
}

function buildUrl(path: string): string {
  const base = getApiBase().replace(/\/$/, "");
  // NOTE(coral-ui frontend): caller passes absolute API paths starting
  // with `/`, e.g. `/pages` — we glue to `apiBase` (`/api/v1`). Special
  // case `/health` lives outside `/api/v1` so we accept absolute paths
  // starting with `/health` and route to origin.
  if (path.startsWith("/health")) return path;
  return `${base}${path.startsWith("/") ? path : `/${path}`}`;
}

export async function api<T>(path: string, opts: ApiOptions = {}): Promise<T> {
  const url = buildUrl(path);
  const headers = new Headers(opts.headers ?? {});
  if (opts.body !== undefined && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  const token = useAuthStore.getState().token;
  if (token && opts.auth !== false) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  let res: Response;
  try {
    res = await fetch(url, {
      method: opts.method ?? (opts.body ? "POST" : "GET"),
      headers,
      body:
        opts.body === undefined
          ? undefined
          : typeof opts.body === "string"
            ? opts.body
            : JSON.stringify(opts.body),
      signal: opts.signal ?? undefined,
      credentials: "same-origin",
    });
  } catch (e) {
    throw new ApiError(
      "NETWORK",
      e instanceof Error ? e.message : String(e),
      0,
    );
  }

  if (!res.ok) {
    let envelope: Partial<ErrorEnvelope> = {};
    try {
      envelope = (await res.json()) as ErrorEnvelope;
    } catch {
      // body wasn't JSON; fall through to defaults
    }
    throw new ApiError(
      envelope.error?.code ?? "UNKNOWN",
      envelope.error?.message ?? res.statusText ?? "request failed",
      res.status,
      envelope.error?.hint,
    );
  }

  if (res.status === 204) return undefined as T;

  const json = await res.json();
  if (opts.raw) return json as T;
  return (json?.data ?? json) as T;
}

/**
 * Build a query string from a typed record. Skips undefined / null / "" values
 * and joins arrays with commas (the backend accepts CSV for multi-select).
 */
export function qs(
  params: Record<
    string,
    string | number | boolean | string[] | null | undefined
  >,
): string {
  const out = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === "") continue;
    if (Array.isArray(v)) {
      if (v.length === 0) continue;
      out.set(k, v.join(","));
    } else {
      out.set(k, String(v));
    }
  }
  const s = out.toString();
  return s ? `?${s}` : "";
}
