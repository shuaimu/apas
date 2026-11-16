"use client";

import { useState, type ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { overviewMuted } from "@/components/overview/OverviewElements";

/**
 * A part of the page that stays closed until asked for. Its content is not
 * mounted while closed, so a closed editor sends no request. Closed on every
 * visit: what is inside is rarely changed.
 */
export function Disclosure({ id, label, description, children }: {
  id: string;
  label: string;
  description?: string;
  children: () => ReactNode;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className="min-w-0">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-start gap-2 rounded px-4 py-3 text-left transition-colors hover:bg-gray-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 dark:hover:bg-gray-700/40"
      >
        <ChevronRight
          aria-hidden="true"
          className={`mt-0.5 h-4 w-4 shrink-0 text-gray-500 transition-transform dark:text-gray-400 ${open ? "rotate-90" : ""}`}
        />
        <span className="min-w-0">
          <span className="block text-sm font-medium text-gray-900 dark:text-gray-100">{label}</span>
          {description && <span className={`block ${overviewMuted}`}>{description}</span>}
        </span>
      </button>
      <div id={id} hidden={!open} className="px-4 pb-4">
        {open && children()}
      </div>
    </div>
  );
}
