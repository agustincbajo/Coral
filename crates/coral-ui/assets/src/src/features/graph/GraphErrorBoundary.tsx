import { Component, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

// Detect WebGL 2 support. Sigma.js v3 requires WebGL 2 (breaking change
// vs v2). Without WebGL 2 the Sigma constructor throws synchronously
// during the first render, which (with no Error Boundary) propagates
// up the React tree and unmounts the entire SPA — leaving a blank page.
// We detect explicitly so we can render a friendly fallback instead.
export function hasWebGL2(): boolean {
  try {
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2");
    return gl !== null && gl !== undefined;
  } catch {
    return false;
  }
}

interface State {
  hasError: boolean;
  error: Error | null;
}

interface Props {
  fallback: ReactNode;
  children: ReactNode;
}

// Local Error Boundary scoped to the Sigma graph. Anything that throws
// inside `<GraphCanvas>` is caught here so it cannot derail the rest of
// the app (header nav, sidebar, other routes).
export class GraphErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: unknown): void {
    // eslint-disable-next-line no-console
    console.error("[GraphErrorBoundary]", error, info);
  }

  render(): ReactNode {
    if (this.state.hasError) {
      return this.props.fallback;
    }
    return this.props.children;
  }
}

// Friendly fallback for either (a) WebGL 2 missing on this device, or
// (b) Sigma threw something else at render time. Keeps the rest of the
// SPA usable and points the user at the alternative views.
export function GraphFallback({ reason }: { reason: "no-webgl2" | "render-error" }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-6 text-sm space-y-2">
      <div className="font-medium">
        {reason === "no-webgl2"
          ? t("graph.fallback.no_webgl2_title")
          : t("graph.fallback.render_error_title")}
      </div>
      <p className="text-muted-foreground text-xs">
        {reason === "no-webgl2"
          ? t("graph.fallback.no_webgl2_body")
          : t("graph.fallback.render_error_body")}
      </p>
    </div>
  );
}
