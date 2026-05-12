// Minimal toaster — Zustand store + a fixed-position list of cards.
// Avoids pulling Radix Toast (which would add ~12 KB) for the M1
// notification surface; we only need ephemeral text + a close button.
//
// Usage:
//   import { useToast } from "@/components/ui/toaster";
//   const toast = useToast();
//   toast({ title: "Saved", description: "...", variant: "success" });
//
// Mount `<Toaster />` once at the app root.

import { useEffect } from "react";
import { create } from "zustand";
import { X } from "lucide-react";
import { cn } from "@/lib/utils";

export type ToastVariant = "default" | "success" | "error" | "warning";

interface Toast {
  id: string;
  title: string;
  description?: string;
  variant: ToastVariant;
  ttlMs: number;
}

interface ToastState {
  toasts: Toast[];
  push: (t: Omit<Toast, "id" | "ttlMs"> & { ttlMs?: number }) => string;
  dismiss: (id: string) => void;
  clear: () => void;
}

const useToastStore = create<ToastState>((set) => ({
  toasts: [],
  push: ({ ttlMs = 5000, variant = "default", title, description }) => {
    const id =
      typeof crypto !== "undefined" && crypto.randomUUID
        ? crypto.randomUUID()
        : `t-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    set((s) => ({
      toasts: [...s.toasts, { id, title, description, variant, ttlMs }],
    }));
    return id;
  },
  dismiss: (id) =>
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
  clear: () => set({ toasts: [] }),
}));

/**
 * Hook to push a new toast from anywhere in the tree. Returns the
 * `push` callback directly so consumers can `toast({...})`.
 */
export function useToast() {
  return useToastStore((s) => s.push);
}

function ToastItem({ toast }: { toast: Toast }) {
  const dismiss = useToastStore((s) => s.dismiss);
  useEffect(() => {
    if (toast.ttlMs <= 0) return;
    const id = setTimeout(() => dismiss(toast.id), toast.ttlMs);
    return () => clearTimeout(id);
  }, [toast.id, toast.ttlMs, dismiss]);
  return (
    <div
      role="status"
      className={cn(
        "rounded-lg border bg-background p-3 pr-9 shadow-md text-sm relative w-80",
        toast.variant === "success" && "border-emerald-500/40",
        toast.variant === "error" && "border-destructive/60",
        toast.variant === "warning" && "border-amber-500/50",
      )}
    >
      <button
        type="button"
        onClick={() => dismiss(toast.id)}
        className="absolute right-2 top-2 text-muted-foreground hover:text-foreground"
        aria-label="dismiss"
      >
        <X className="h-4 w-4" />
      </button>
      <div className="font-medium">{toast.title}</div>
      {toast.description ? (
        <div className="text-xs text-muted-foreground mt-1">
          {toast.description}
        </div>
      ) : null}
    </div>
  );
}

export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  if (toasts.length === 0) return null;
  return (
    <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} />
      ))}
    </div>
  );
}
