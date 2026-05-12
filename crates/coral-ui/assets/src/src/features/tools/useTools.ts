import { useMutation } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type {
  DownToolInput,
  RunTestToolInput,
  ToolRunResult,
  UpToolInput,
  VerifyToolInput,
} from "@/lib/types";

// NOTE(coral-ui frontend): each mutation hits a distinct endpoint —
// we keep the body shapes typed so the route components can pass
// arrays directly instead of CSV-parsing twice.

export function useVerify() {
  return useMutation<ToolRunResult, Error, VerifyToolInput>({
    mutationFn: (body) =>
      api<ToolRunResult>("/tools/verify", { method: "POST", body }),
  });
}

export function useRunTest() {
  return useMutation<ToolRunResult, Error, RunTestToolInput>({
    mutationFn: (body) =>
      api<ToolRunResult>("/tools/run_test", { method: "POST", body }),
  });
}

export function useUp() {
  return useMutation<ToolRunResult, Error, UpToolInput>({
    mutationFn: (body) =>
      api<ToolRunResult>("/tools/up", { method: "POST", body }),
  });
}

export function useDown() {
  return useMutation<ToolRunResult, Error, DownToolInput>({
    mutationFn: (body) =>
      api<ToolRunResult>("/tools/down", { method: "POST", body }),
  });
}
