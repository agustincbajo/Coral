import { useCallback, useRef, useState } from "react";
import { getApiBase } from "@/lib/config";
import { useAuthStore } from "@/stores/auth";
import { useQueryHistory, type QueryMode } from "@/stores/query";

// NOTE(coral-ui frontend): EventSource does not support POST + headers,
// so we hand-roll an SSE consumer on top of fetch + ReadableStream.

interface FrameHandler {
  onToken: (id: string, text: string) => void;
  onSource: (id: string, slug: string) => void;
  onDone: (id: string) => void;
  onError: (id: string, message: string) => void;
}

function parseFrames(buf: string): { frames: { event: string; data: string }[]; rest: string } {
  // Frames are separated by double newlines (\n\n). Each frame is a list
  // of `key: value` lines.
  const frames: { event: string; data: string }[] = [];
  const blocks = buf.split(/\n\n/);
  const rest = blocks.pop() ?? "";
  for (const block of blocks) {
    const lines = block.split(/\n/);
    let event = "message";
    const dataLines: string[] = [];
    for (const line of lines) {
      if (line.startsWith("event:")) event = line.slice(6).trim();
      else if (line.startsWith("data:")) dataLines.push(line.slice(5).trimStart());
    }
    if (dataLines.length) frames.push({ event, data: dataLines.join("\n") });
  }
  return { frames, rest };
}

export function useQueryStream() {
  const abortRef = useRef<AbortController | null>(null);
  const [streaming, setStreaming] = useState(false);

  const send = useCallback(
    async (question: string, mode: QueryMode, model?: string) => {
      const trimmed = question.trim();
      if (!trimmed) return;
      const id = crypto.randomUUID();
      const turn = {
        id,
        question: trimmed,
        mode,
        answer: "",
        sources: [] as string[],
        status: "pending" as const,
      };
      useQueryHistory.getState().push(turn);

      const handlers: FrameHandler = {
        onToken: useQueryHistory.getState().appendToken,
        onSource: useQueryHistory.getState().addSource,
        onDone: (i) => useQueryHistory.getState().finish(i, "done"),
        onError: (i, m) => useQueryHistory.getState().finish(i, "error", m),
      };

      const ac = new AbortController();
      abortRef.current?.abort();
      abortRef.current = ac;
      setStreaming(true);

      const base = getApiBase().replace(/\/$/, "");
      const token = useAuthStore.getState().token;
      const headers: Record<string, string> = {
        "content-type": "application/json",
        accept: "text/event-stream",
      };
      if (token) headers["Authorization"] = `Bearer ${token}`;

      try {
        const res = await fetch(`${base}/query`, {
          method: "POST",
          headers,
          body: JSON.stringify({ q: trimmed, mode, ...(model ? { model } : {}) }),
          signal: ac.signal,
        });
        if (!res.ok) {
          let msg = res.statusText;
          try {
            const j = await res.json();
            msg = j?.error?.message ?? msg;
          } catch {
            // body wasn't JSON
          }
          handlers.onError(id, msg);
          return;
        }
        if (!res.body) {
          handlers.onError(id, "missing response body");
          return;
        }
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buf = "";
        while (true) {
          const { done, value } = await reader.read();
          if (done) break;
          buf += decoder.decode(value, { stream: true });
          const { frames, rest } = parseFrames(buf);
          buf = rest;
          for (const f of frames) {
            if (f.event === "token") {
              try {
                const payload = JSON.parse(f.data) as { text?: string };
                if (payload.text) handlers.onToken(id, payload.text);
              } catch {
                handlers.onToken(id, f.data);
              }
            } else if (f.event === "source") {
              try {
                const payload = JSON.parse(f.data) as { slug?: string };
                if (payload.slug) handlers.onSource(id, payload.slug);
              } catch {
                // ignore malformed source frames
              }
            } else if (f.event === "done") {
              handlers.onDone(id);
            } else if (f.event === "error") {
              try {
                const payload = JSON.parse(f.data) as { message?: string };
                handlers.onError(id, payload.message ?? "stream error");
              } catch {
                handlers.onError(id, f.data || "stream error");
              }
            }
          }
        }
        handlers.onDone(id);
      } catch (e) {
        if ((e as { name?: string })?.name === "AbortError") return;
        handlers.onError(id, e instanceof Error ? e.message : String(e));
      } finally {
        setStreaming(false);
      }
    },
    [],
  );

  const cancel = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setStreaming(false);
  }, []);

  return { send, cancel, streaming };
}
