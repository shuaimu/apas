## Context

See `proposal.md` for motivation and `specs/pane-toolbar/spec.md` for observable behavior. Today `TabbedView` owns the shared toolbar, including Timeline/Chat, provider, model, and effort controls. A nested terminal-pane component separately owns the persisted Terminal/Conversation mode and renders its toggle as the first row of pane content. Moving that toggle into the shared toolbar requires the toolbar and terminal body to use one mode value without violating React hook ordering as panes are added or removed.

The terminal body must remain mounted while Conversation is selected so its xterm instance retains scrollback, focus, and attachment state. Pane model and effort values are also part of persisted pane configuration and existing WebSocket contracts, even though their inline selectors are being removed.

## Goals / Non-Goals

**Goals:**

- Give the toolbar and active terminal body a single source of truth for the persisted Terminal/Conversation mode.
- Preserve terminal mounting and conversation-input behavior while relocating only the switch presentation.
- Remove obsolete timeline and inline model/effort UI code without changing unrelated controls.
- Keep the change confined to the web client and its tests.

**Non-Goals:**

- Removing model or effort fields, store actions, WebSocket messages, CLI flags, or persisted values.
- Removing or redesigning the provider selector.
- Changing terminal transcript capture, usage calculation, terminal input transport, or the provider TUI's model/effort commands.
- Adding a replacement timeline experience.

## Decisions

### Hoist active terminal view state to the toolbar owner

`TabbedView` will obtain the active pane's mode through the existing per-session/per-pane persistence hook at a stable hook call site. It will render `TerminalViewToggle` in the toolbar for an active terminal pane and pass the same controlled mode into the terminal content component.

The terminal content component will no longer render its own toggle or own a second copy of mode state. It will continue to hide rather than unmount the live terminal while showing the structured conversation.

This is preferred to rendering the toggle through a portal from each mounted terminal child. A portal would preserve local ownership but would introduce a toolbar DOM target, active-pane arbitration among multiple mounted children, and additional lifecycle edge cases.

### Place the terminal switch directly before usage

The conditional terminal switch will be emitted immediately before the conditional provider-usage block in toolbar document order. The toggle component's presentation will be adjusted so it fits the toolbar rather than drawing a second full-width pane-content row. If usage is unavailable, the switch remains in the same toolbar control group.

This preserves the toolbar's existing wrapping behavior on narrow displays while satisfying both visual and accessible reading order.

### Remove the timeline path completely from the pane view

`TabbedView` will stop tracking timeline-enabled pane ids and will always render the standard `MessagePane` path for non-terminal panes. The timeline component, imports, and extraction utility/tests will be removed when repository search confirms they have no other consumers.

Keeping an unreachable timeline implementation was considered, but it would leave dead state and rendering code that can drift without coverage.

### Remove only the inline model and effort presentation

The toolbar model and reasoning-effort `<select>` elements and UI-only option/normalization code that becomes unused will be removed. Provider switching remains because it is a distinct requested action. Existing stored values and store/server/CLI actions remain intact; selecting or activating a pane must not mutate them. Bot startup will continue to use the pane's saved effort behavior rather than silently resetting it because the selector disappeared.

### Cover behavior at the TabbedView boundary

Component tests will verify that:

- terminal panes expose the switch in the toolbar and it precedes Codex usage;
- switching views changes content while retaining the same mounted terminal instance;
- per-pane persisted selections are restored across tab changes;
- non-terminal panes do not expose the terminal switch or timeline action;
- model and effort selectors are absent while provider and other applicable actions remain.

Tests dedicated only to removed timeline behavior will be deleted with that feature.

## Risks / Trade-offs

- **[Risk] Changing the active pane can briefly expose the previous pane's view mode while persisted state is restored.** → Key mode synchronization by session and pane and add a tab-switch restoration test; avoid maintaining independent toolbar and content state.
- **[Risk] Moving the switch into a wrapping toolbar can separate it visually from usage on very narrow screens.** → Keep the two blocks adjacent in DOM order and use compact, non-full-width toggle styling.
- **[Risk] Removing inline selectors reduces discoverability for users accustomed to toolbar configuration.** → This is an intentional product trade-off; preserve stored configuration and provider switching so the removal is presentation-only.
- **[Risk] Timeline helpers could have an unrecognized consumer.** → Confirm references before deletion and retain the utility if another consumer exists, while still removing the pane toolbar and rendering path.

## Migration Plan

No data migration is required. Deploy the web-client change normally; existing local terminal view preferences and pane model/effort values remain valid. Rollback consists of restoring the previous web build because no persisted schema or protocol changes are introduced.
