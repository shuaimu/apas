"use client";

import { useEffect } from "react";
import { CheckCircle, Info, XCircle } from "lucide-react";
import { useStore, type Toast } from "@/lib/store";

const TOAST_TTL_MS = 3000;

function ToastItem({ toast }: { toast: Toast }) {
  const dismissToast = useStore((s) => s.dismissToast);

  useEffect(() => {
    const t = setTimeout(() => dismissToast(toast.id), TOAST_TTL_MS);
    return () => clearTimeout(t);
  }, [toast.id, dismissToast]);

  const styles =
    toast.kind === "success"
      ? "bg-green-500 text-white border-green-600"
      : toast.kind === "error"
        ? "bg-red-500 text-white border-red-600"
        : "bg-gray-800 text-white border-gray-700";

  const icon =
    toast.kind === "success" ? (
      <CheckCircle className="w-5 h-5 flex-shrink-0" />
    ) : toast.kind === "error" ? (
      <XCircle className="w-5 h-5 flex-shrink-0" />
    ) : (
      <Info className="w-5 h-5 flex-shrink-0" />
    );

  return (
    <div
      className={`flex items-center gap-2 px-4 py-3 rounded-lg shadow-lg border ${styles} animate-[slideIn_0.2s_ease-out]`}
      role="status"
    >
      {icon}
      <span className="text-sm font-medium">{toast.message}</span>
      <button
        onClick={() => dismissToast(toast.id)}
        className="ml-2 opacity-70 hover:opacity-100"
        aria-label="Dismiss"
      >
        <XCircle className="w-4 h-4" />
      </button>
    </div>
  );
}

export function Toaster() {
  const toasts = useStore((s) => s.toasts);

  if (toasts.length === 0) return null;

  return (
    <div className="fixed top-4 right-4 z-50 flex flex-col gap-2 pointer-events-auto">
      {toasts.map((t) => (
        <ToastItem key={t.id} toast={t} />
      ))}
    </div>
  );
}
