import Link from "next/link";

export function WorkersNav() {
  return <nav aria-label="Autonomous workers" className="flex shrink-0 flex-wrap gap-2 px-3 py-2">
    <Link href="/code/workers" className="inline-flex min-h-11 min-w-11 items-center rounded-lg border border-gray-300 px-3 text-sm font-semibold hover:bg-gray-100 focus-visible:outline-2 dark:border-gray-600 dark:hover:bg-gray-800">Workers</Link>
    <Link href="/code/workers?view=attention" className="inline-flex min-h-11 min-w-11 items-center rounded-lg border border-gray-300 px-3 text-sm font-semibold hover:bg-gray-100 focus-visible:outline-2 dark:border-gray-600 dark:hover:bg-gray-800">Attention</Link>
  </nav>;
}
