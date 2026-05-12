import { Navigate, Route, Routes } from "react-router-dom";
import { Layout } from "@/components/Layout";
import { PagesList } from "@/routes/PagesList";
import { PageDetail } from "@/routes/PageDetail";
import { GraphView } from "@/routes/GraphView";
import { QueryPlayground } from "@/routes/QueryPlayground";
import { ManifestView } from "@/routes/ManifestView";

export function App() {
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
        <Route path="*" element={<Navigate to="/pages" replace />} />
      </Route>
    </Routes>
  );
}
