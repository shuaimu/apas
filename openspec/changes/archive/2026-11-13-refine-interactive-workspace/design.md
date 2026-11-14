## Context

See `proposal.md` for the motivation. This document records the decisions that are not obvious from the requirements, and the order of deployment. The work is in the tree; nothing is deployed.

Three facts shape the approach:

- Pi was a supported provider, so its value is in `.apas` files, stored rosters, stored policies and in older clients' frames. All of that must keep loading.
- OMP is derived from Pi. It writes the same session record format and its name contains the letters "pi". Retiring Pi must not catch it.
- Autopilot's start request carries a prompt and nothing about where the prompt came from. The tab, the CLI and the server validate and run it as text.

## Goals / Non-Goals

**Goals:**

- One tap from "+" to a provider list.
- Pi cannot run anywhere, and every refusal says what to use instead.
- An Overview that fits about a screen and a half and starts with what is running.
- A useful prompt in the Start autopilot dialog without a new wire field.

**Non-Goals:**

- Converting a saved Pi tab into an OMP tab. Their session stores differ, and a silent provider change is what the retired-provider rules forbid.
- Deleting saved Pi tabs. They are the user's history; a person closes them.
- Changing autonomous worker creation, approval or execution.
- Changing the native app. It has no Pi entry and no chooser, and its launch profiles come from the server.
- Changing what an autopilot does once started.

## Decisions

### 1. Pi joins the existing retired-provider pattern

Pi is retired the way MiniMax and GLM were: the provider value stays as a decode-only value, the launch profile leaves the catalog, and server and CLI refuse independently. The one difference is the sentence. A retired backend is answered with "Unsupported provider: this backend has been retired"; Pi is answered with "Pi is no longer supported. Use OMP instead.", because there is something the person can do.

Alternative rejected: delete the `pi` value. Every `.apas` file and stored roster that holds a Pi tab would fail to parse, and an older client's frame would drop its connection.

Alternative rejected: map Pi to OMP on load. The tab would look usable while its conversation stayed in a store OMP does not read.

### 2. The Pi capability is deleted, not kept as "never satisfied"

`terminal_pi_v1` gated routing a Pi launch to a CLI that could host it. With no Pi launch there is nothing to gate. The server refuses a Pi request before it looks at capabilities, so an older CLI that still advertises the capability changes nothing.

### 3. A saved Pi tab is reported, never started, and keeps no pane host

The CLI lists the tab, so it can be read and closed, and reports its terminal as exited with the retirement sentence as the status. The web words the notice from that state. The CLI does not write the sentence into the tab's conversation: it would be stored again at every project start.

The CLI leaves a retired provider's tab out of the pane hosts it keeps. A Pi agent still running under a pane host when its machine is updated is therefore ended when the updated CLI starts the project, not after the reboot lease. Without this, the agent would run on where APAS can neither show nor stop it.

A saved Pi tab still blocks the project's Reboot CLI, like any retired pane, and the refusal names the tab. The Machines page restart is not blocked.

### 4. The server refuses input to a retired tab

Typed input to a tab with no process used to be passed to the CLI, stored, and shown as a message no agent answers. The server now refuses it before storing anything. Raw keystrokes are dropped silently: an error for each key would flood the client, and the tab's terminal state already says why. A switch of a saved Pi tab's provider is refused before the roster is edited, so viewers never see an OMP tab that does not exist.

### 5. The Template control is derived from the prompt text

The control has no state of its own. It names a template only while the prompt field holds that template's exact text, and reads "My own text" otherwise. So the control cannot say one thing while the dialog sends another, and the start request needs no new field.

Choosing a template changes the prompt only. It does not tick or clear "End when an iteration makes no tool call". A line under the control says what the combination means instead. Alternative rejected: let a template set the option. A person who had cleared it on purpose would have it ticked again by a choice that looked like it was about text.

Each template describes one step, because the same text is submitted on every iteration. A template that can run out of work ends in two turns: one turn finds nothing left and says so with a fixed sentence; the next sees that sentence and calls no tool, which ends the run when the option is ticked.

### 6. "+" loses the chooser; the Overview gains the entry

The chooser cost one step on every new tab to offer something that is off unless a deployment enables and qualifies it. Worker creation needs a review of mandate, host and allowance, which the Overview's Autonomous workers section and `/code/workers` already lead to.

### 7. One Overview layout for desktop and phone

The worker workspace renders the Overview in one layout on the desktop and on the phone: the project's name, the caller's panels, then Autonomous workers, all built from the Overview page's shared small parts. The desktop's panels are Tabs, Usage, Resource use and Allowed launch profiles. The phone's panel is Tabs, one row per tab. The composer, a worker's detail and an earlier result keep the Jobs style, and keep "Back to interactive agents", because they are pages of their own.

## Risks / Trade-offs

- Risk: a stored project override that listed only the Pi profile becomes an empty allowlist, which permits no launch. Mitigation: this is the existing rule for retired profiles. It is explicit, and an administrator can change the override. The deployment task checks for it after the first server start.
- Risk: updating a machine ends a Pi agent that an older CLI left running. Mitigation: intended; see decision 3. The work was done on the statement that no tab of this deployment uses Pi. Production was not checked for one.
- Risk: a template's wording has not been run on a real OMP. Mitigation: none yet. Unit tests assert the text and its validation only, and the open tasks list the real run.
- Risk: the no-tool-call ending depends on the model following two fixed sentences. Mitigation: the dialog says for each template how its run ends, both limits stay available, and "Keep the tests green" says it has no end of its own.
- Risk: an older web bundle still offers Pi and the chooser until it is reloaded. Mitigation: the server refuses Pi with the retirement sentence, and the chooser's worker entry opens the same composer as before.

## Migration Plan

1. Deploy the server. On its first start it drops `terminal:pi:official:default` from stored policies and gives each changed policy a newer version.
2. Deploy the web and protocol pair.
3. Push the CLI. A machine takes it at its next reboot from the Machines page, which stays its owner's action.

Rollback: redeploy the older web, then the older server. The Pi profile is not put back into stored policies; an administrator adds it again if Pi is wanted. An older CLI needs no rollback step: it ignores what it does not know.
