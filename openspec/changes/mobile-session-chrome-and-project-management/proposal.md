## Why

The mobile conversation screen spends three rows on chrome before any of the
conversation is visible: a title row, a row of actions, and a row of pane chips.
On a phone that is most of the first screenful, and two of those rows hold
controls that are not used mid-conversation — Account, which belongs on the home
screen, and Raw terminal and Summary, which are occasional.

The pane list is the opposite: it is how you move between panes, it is used
constantly, and it sits third.

Separately, a project's allowed tab types can be set from nowhere. The deny list
exists, the CLI enforces it on every `AddPane`, and the web has the helpers to
edit it — but no interface anywhere reads them, so an owner cannot restrict what
their project may create without editing `.apas` by hand.

## What Changes

- **The pane list moves into the top row**, immediately right of the back
  button, with the add-pane control alongside it. Project name, host, and status
  move to a single line beneath. Chrome above the conversation goes from three
  rows to two.
- **The Account button leaves the conversation screen.** It navigates away from
  the session, and the home screen already has it.
- **Raw terminal and Summary move next to the send button**, as icons, where
  occasional per-pane actions belong and where they cost no vertical space.
- **A Manage button takes the Account button's place**, opening project
  management for the current project.
- **Project management on mobile can set the tab types the project allows** —
  the first interface for this on any surface. Presented as an allow list over
  the stored deny list, so a project that has never been restricted shows
  everything permitted.
- **Only an owner may change it.** Everyone else sees the same list, read-only,
  which is how the desktop already treats project settings.

## Capabilities

### Modified Capabilities

- `mobile-code-sessions`: the session screen's controls are rearranged around
  what is used during a conversation, and mobile gains project management for
  the tab types a project allows, restricted to those who may manage it.

## Impact

- `packages/web/src/components/mobile/MobileSessionActivity.tsx`: header
  restructured; Account removed; Raw terminal and Summary move to the compose
  row; Manage added.
- `packages/web/src/app/page.tsx`: the session screen no longer needs an account
  callback.
- A new mobile project-management sheet, reusing `updateProjectFlags`,
  `ALL_TAB_TYPES`, and `useCanManageCurrentProject` — all of which exist, with
  `ALL_TAB_TYPES` currently referenced by no component at all.
- No server, protocol, or CLI change: `disallowed_tab_types` already travels
  both ways and the CLI already enforces it.
