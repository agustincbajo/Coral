import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeSanitize from "rehype-sanitize";

// NOTE(coral-ui frontend): Mermaid was removed from M1 to stay within
// the binary size budget (~2.7 MB savings). Mermaid fences are rendered
// as plain code blocks with a tag. M2 will restore Mermaid via lazy
// CDN load when the user opts in.

const MERMAID_FENCE = /```mermaid\n([\s\S]*?)```/;

function MermaidPlaceholder({ source }: { source: string }) {
  return (
    <div className="my-3 rounded border border-dashed border-muted-foreground/30 bg-muted/30 p-3">
      <div className="mb-2 text-xs uppercase tracking-wide text-muted-foreground">
        mermaid diagram (rendering coming in M2)
      </div>
      <pre className="overflow-x-auto whitespace-pre text-xs text-foreground/80">
        {source}
      </pre>
    </div>
  );
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
              return <MermaidPlaceholder source={content} />;
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
