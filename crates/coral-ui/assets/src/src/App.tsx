import { Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "@/components/Layout";
import { PagesList } from "@/routes/PagesList";
import { PageDetail } from "@/routes/PageDetail";
import { GraphView } from "@/routes/GraphView";
import { QueryPlayground } from "@/routes/QueryPlayground";
import { ManifestView } from "@/routes/ManifestView";
import { InterfacesView } from "@/routes/InterfacesView";
import { DriftView } from "@/routes/DriftView";
import { AffectedView } from "@/routes/AffectedView";
import { ToolsView } from "@/routes/ToolsView";
import { GuaranteeView } from "@/routes/GuaranteeView";
import { useWikiEvents } from "@/lib/useWikiEvents";

export function App() {
  // SSE hook mounted at the root — single stream for the whole SPA.
  useWikiEvents();
  return (
    <Routes>
      <Route element={<Layout />}>
        <Route path="/" element={<Navigate to="/pages" replace />} />
        <Route path="/pages" element={<PagesList />} />
        <Route path="/pages/:repo/:slug" element={<PageDetail />} />
        <Route path="/pages/:slug" element={<PageDetail />} />
        <Route path="/graph" element={<GraphView />} />
        <Route path="/query" element={<QueryPlayground />} />
        <Route path="/manifest" element={<ManifestView />} />
        <Route path="/interfaces" element={<InterfacesView />} />
        <Route path="/drift" element={<DriftView />} />
        <Route path="/affected" element={<AffectedView />} />
        <Route path="/tools" element={<ToolsView />} />
        <Route path="/guarantee" element={<GuaranteeView />} />
        <Route path="*" element={<Navigate to="/pages" replace />} />
      </Route>
    </Routes>
  );
}
