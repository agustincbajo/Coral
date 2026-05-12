import { useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeSanitize from "rehype-sanitize";

// NOTE(coral-ui v0.32.2): Mermaid is lazy-loaded from a CDN only when a
// page body actually contains a ```mermaid``` fence. Keeps the binary
// (and the offline-only baseline bundle) small while still rendering
// diagrams when the user is online and explicitly authors one. The CDN
// URL is pinned by SHA-tagged version; a network or CSP failure falls
// back gracefully to a labeled `<pre>` showing the source.

const MERMAID_FENCE = /```mermaid\n([\s\S]*?)```/;
const MERMAID_CDN_URL =
  "https://cdn.jsdelivr.net/npm/mermaid@11.4.0/dist/mermaid.esm.min.mjs";

interface MermaidLib {
  initialize: (opts: Record<string, unknown>) => void;
  render: (id: string, src: string) => Promise<{ svg: string }>;
}

let mermaidPromise: Promise<MermaidLib | null> | null = null;
function loadMermaid(): Promise<MermaidLib | null> {
  if (!mermaidPromise) {
    mermaidPromise = (async () => {
      try {
        // @vite-ignore — vite would normally try to bundle this; we
        // explicitly want a runtime import from a CDN URL.
        const mod = await import(/* @vite-ignore */ MERMAID_CDN_URL);
        const m = (mod.default ?? mod) as MermaidLib;
        m.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: "neutral",
        });
        return m;
      } catch (e) {
        // eslint-disable-next-line no-console
        console.warn("[Coral UI] mermaid CDN load failed:", e);
        return null;
      }
    })();
  }
  return mermaidPromise;
}

function MermaidBlock({ source }: { source: string }) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const id = useMemo(
    () => `mmd-${Math.random().toString(36).slice(2, 9)}`,
    [],
  );
  useEffect(() => {
    let cancelled = false;
    void loadMermaid()
      .then((m) => {
        if (!m) {
          if (!cancelled) setError("offline");
          return null;
        }
        return m.render(id, source);
      })
      .then((res) => {
        if (res && !cancelled && ref.current) ref.current.innerHTML = res.svg;
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [id, source]);

  if (error) {
    return (
      <div className="my-3 rounded border border-dashed border-muted-foreground/30 bg-muted/30 p-3">
        <div className="mb-2 text-xs uppercase tracking-wide text-muted-foreground">
          mermaid diagram (
          {error === "offline"
            ? "CDN unreachable — source shown"
            : `render failed: ${error}`}
          )
        </div>
        <pre className="overflow-x-auto whitespace-pre text-xs text-foreground/80">
          {source}
        </pre>
      </div>
    );
  }
  return <div ref={ref} className="my-3 [&_svg]:max-w-full" />;
}

export function MarkdownRenderer({ source }: { source: string }) {
  return (
    <div className="prose prose-slate dark:prose-invert max-w-none">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeSanitize]}
        components={{
          code({ className, children, ...props }) {
            const lang = /language-(\w+)/.exec(className || "")?.[1];
            const content = String(children ?? "").replace(/\n$/, "");
            if (lang === "mermaid") {
              return <MermaidBlock source={content} />;
            }
            return (
              <code className={className} {...props}>
                {children}
              </code>
            );
          },
          a({ href, children, ...props }) {
            const isExternal = !!href && /^https?:/i.test(href);
            return (
              <a
                href={href}
                target={isExternal ? "_blank" : undefined}
                rel={isExternal ? "noopener noreferrer" : undefined}
                {...props}
              >
                {children}
              </a>
            );
          },
        }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}

// Helper: detect a mermaid fence quickly without parsing.
export function hasMermaid(src: string): boolean {
  return MERMAID_FENCE.test(src);
}
