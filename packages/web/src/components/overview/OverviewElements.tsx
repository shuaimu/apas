import type { ReactNode } from "react";

/**
 * The Overview's one set of parts: a section with its heading, a card, a
 * button and an empty state. Every panel of the page is built from these, so
 * the page reads as one thing. The button is the tab bar's small button, since
 * the Overview sits right under that bar.
 */

/** The page's column. Every section shares this width and gutter. */
export const overviewPage = "mx-auto flex w-full max-w-6xl flex-col gap-8 px-4 pb-10 pt-5 sm:px-6";

export const overviewCard = "rounded-lg border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800/40";

// The shape every bordered button shares: size, border, type and focus ring.
// Each tone adds only its colours, so a tone never carries two hover colours
// and leaves the winner to chance. With a mouse it is the tab bar's small
// button. On a touch screen it is taller (40px), so a finger can hit it on the
// phone's Overview.
const buttonShape = "inline-flex h-8 pointer-coarse:h-10 pointer-coarse:px-3 shrink-0 items-center justify-center gap-1 whitespace-nowrap rounded border";
const buttonType = "px-2.5 text-xs font-medium";
const buttonFocus = "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 disabled:cursor-not-allowed disabled:opacity-50";

export const overviewButton = `${buttonShape} border-gray-300 bg-white ${buttonType} text-gray-700 transition-colors hover:bg-gray-50 ${buttonFocus} disabled:hover:bg-white dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700 dark:disabled:hover:bg-gray-800`;

/** The one button a form or dialog is for, such as Save or New project. */
export const overviewPrimaryButton = `${buttonShape} ${buttonType} transition-colors ${buttonFocus} border-blue-600 bg-blue-600 text-white hover:bg-blue-700 disabled:hover:bg-blue-600 dark:border-blue-500 dark:bg-blue-600 dark:hover:bg-blue-500 dark:disabled:hover:bg-blue-600`;

/** A bordered button for an action that stops or ends something. */
export const overviewDangerButton = `${buttonShape} ${buttonType} transition-colors ${buttonFocus} border-red-300 bg-white text-red-700 hover:bg-red-50 dark:border-red-800 dark:bg-gray-800 dark:text-red-300 dark:hover:bg-red-950/40`;

// A button that reads as text until it is pointed at: for the action a card
// must offer but should not shout. The tone is kept apart from the shape so a
// variant never carries two hover colours and leaves the winner to chance.
const quietButtonShape = "inline-flex h-7 shrink-0 items-center justify-center gap-1 whitespace-nowrap rounded px-2 text-xs font-medium text-gray-500 transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-500 disabled:cursor-not-allowed disabled:opacity-50 dark:text-gray-400";

export const overviewQuietButton = `${quietButtonShape} hover:bg-gray-100 hover:text-gray-900 dark:hover:bg-gray-700 dark:hover:text-gray-100`;

/** The quiet button for an action that ends something, such as closing a tab:
 *  as quiet as the others at rest, red when pointed at. */
export const overviewQuietDangerButton = `${quietButtonShape} hover:bg-red-50 hover:text-red-700 dark:hover:bg-red-950/40 dark:hover:text-red-300`;

/** A small label for a kind or a state, such as "Claude". Add `truncate` and
 *  a maximum width where the text is not fixed. */
/** A chip without a text colour, for one that sets its own. Two text-colour
 *  classes on one element do not layer by their order in `className`: the
 *  stylesheet's order decides, and gray came after amber. */
export const overviewChipShape = "shrink-0 rounded bg-gray-100 px-1.5 py-0.5 text-[11px] font-medium dark:bg-gray-700";

export const overviewChip = `${overviewChipShape} text-gray-600 dark:text-gray-300`;

export const overviewMuted = "text-xs text-gray-500 dark:text-gray-400";

interface OverviewSectionProps {
  title: string;
  /** Controls or a quiet note at the right of the heading. */
  aside?: ReactNode;
  /** One quiet line under the heading. */
  description?: ReactNode;
  /** h2 for a section of the page, h3 for a part of one. */
  level?: 2 | 3;
  children: ReactNode;
}

export function OverviewSection({ title, aside, description, level = 2, children }: OverviewSectionProps) {
  const Heading = level === 2 ? "h2" : "h3";
  return (
    <section className="min-w-0">
      <div className="mb-2 flex min-h-8 flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <Heading className="text-sm font-semibold text-gray-900 dark:text-gray-100">{title}</Heading>
        {aside && <div className="flex flex-wrap items-center gap-2">{aside}</div>}
      </div>
      {description && <p className={`-mt-1 mb-3 ${overviewMuted}`}>{description}</p>}
      {children}
    </section>
  );
}

export function OverviewEmpty({ children }: { children: ReactNode }) {
  return (
    <p className="rounded-lg border border-dashed border-gray-300 px-3 py-3 text-sm text-gray-500 dark:border-gray-700 dark:text-gray-400">
      {children}
    </p>
  );
}

/** A notice inside a section: something to know, or with `alert` something
 *  that went wrong. */
export function OverviewNotice({ alert = false, status = false, children }: { alert?: boolean; status?: boolean; children: ReactNode }) {
  return (
    <p
      role={alert ? "alert" : status ? "status" : undefined}
      className={`rounded-lg border px-3 py-2 text-sm ${alert
        ? "border-red-200 bg-red-50 text-red-800 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200"
        : "border-gray-200 bg-gray-50 text-gray-700 dark:border-gray-700 dark:bg-gray-800/40 dark:text-gray-300"}`}
    >
      {children}
    </p>
  );
}
