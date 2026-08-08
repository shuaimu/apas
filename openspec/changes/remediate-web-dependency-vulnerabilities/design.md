## Context

See `proposal.md` for motivation and `specs/web-dependency-security/spec.md` for the release contract. The current npm graph contains 15 vulnerable package entries: direct Next.js, PostCSS, and Vitest dependencies plus vulnerable Babel, AJV, brace-expansion, flatted, js-yaml, minimatch, Nano ID, picomatch, Rollup, Sharp, Vite, and ws resolutions. All findings currently report a fix as available.

The web package has two supported resolution paths. Production deployment runs npm and checks in `package-lock.json`; the manifest declares pnpm 10.28.0 and the Makefile uses pnpm with `pnpm-lock.yaml`. Fixing only the installed `node_modules` tree or one lockfile would leave another supported path vulnerable and allow a later deploy or contributor install to restore the old graph. There is no repository CI workflow today, so verification must be exposed as repeatable local/release commands and documented in the deployment path.

## Goals / Non-Goals

**Goals:**

- Replace every currently vulnerable direct and transitive resolution with a patched, stable release.
- Make npm production installs and pnpm contributor installs deterministic from reviewed lockfiles.
- Prefer parent dependency upgrades and ordinary resolution over long-lived overrides.
- Detect a vulnerable or stale graph before it is declared deployable.
- Preserve the current Next.js application, test suite, and production routes through the upgrade.

**Non-Goals:**

- Auditing or upgrading Rust crates, operating-system packages, nginx, or host-level Node/npm installations.
- Introducing a general automated dependency-update bot or a new repository-wide CI platform.
- Combining unrelated web refactors, visual changes, or feature work with the security update.
- Accepting a permanent vulnerability waiver within this change; any future exception requires separate security review and an explicit specification change.

## Decisions

### 1. Treat npm as the deployment authority while preserving pnpm as a supported contributor path

Production will continue to resolve from `package-lock.json`, but deployment commands will use `npm ci` instead of `npm install` so the reviewed graph cannot change during rollout. The pnpm version declared by `packageManager` remains the contributor toolchain, and `pnpm install --frozen-lockfile` must independently reproduce and audit its checked-in graph.

Removing one lockfile was considered because a single package manager has less drift risk. That would change the established Makefile or the production runbook and is broader than vulnerability remediation, so both remain supported and both become release-gated. The two lockfiles need not be byte-for-byte equivalent, but they must resolve compatible direct versions and both must meet the zero-vulnerability contract.

### 2. Upgrade vulnerable parents first and set patched direct-version floors

Start with stable patched releases of the direct dependency roots. Based on the current advisory ranges, the initial targets are Next.js 16.3.0 or newer stable 16.x, `eslint-config-next` at the matching Next.js line, PostCSS 8.5.26 or newer compatible 8.x, and Vitest 4.1.10 or newer compatible 4.x. Re-resolve both lockfiles so these parent updates can naturally pull patched Sharp, Vite, Rollup, ws, and supporting lint/build dependencies.

The exact selected patch releases must be rechecked against the advisory database and peer/engine constraints during implementation; the fixed floors above prevent a lockfile-free resolver from selecting the known-vulnerable versions again. A blanket `npm audit fix --force` is rejected because it can make unreviewed major upgrades and does not maintain the pnpm graph. Updating every transitive package to its newest major is also rejected because it increases compatibility risk without improving the stated security outcome.

### 3. Use overrides only for vulnerable transitives that parent upgrades cannot dislodge

After ordinary re-resolution, inspect every remaining advisory's dependency path. If a supported parent still constrains a vulnerable transitive despite a compatible patched release being available, add the narrowest package-scoped npm and pnpm override needed to select the patch. Each override must include a short rationale in the verification evidence and must be removable when the parent publishes an adequate constraint.

This is preferred over tolerating a development-only finding: the critical Vitest issue and several file-read/DoS findings live in tooling, and contributor or build environments are still trusted APAS systems. Unconditional top-level pins for all transitives were considered but rejected because they obscure ownership and can violate parent package expectations.

### 4. Make security verification explicit and lockfile-sensitive

Add documented commands or package scripts for low-threshold npm and pnpm audits, plus frozen clean installs. Verification runs from a clean dependency tree and records only advisory/package summaries, never registry credentials. After lockfile regeneration, `npm ci` and `pnpm install --frozen-lockfile` must leave `package.json` and their respective lockfiles unchanged.

The gate uses the package managers' current advisory data rather than a hard-coded list of the original 15 entries, so an advisory published during implementation is also caught. Auditing only production dependencies was rejected because the current critical finding is in Vitest and the web build/test toolchain processes repository and potentially untrusted fixture content.

### 5. Separate dependency verification from the ordinary web build

The dependency audit is a required release check but will not be embedded in `prebuild`. Production builds should not fail merely because the registry advisory service is temporarily unavailable after a graph has already been reviewed. The existing `prebuild` lint gate remains intact; security checks run as an explicit verification step before deployment.

Without a repository CI workflow, the change will add a stable Makefile/package-script entry and update contributor/deployment documentation. A future CI change can invoke the same entry without redesigning the audit process.

### 6. Verify framework-sensitive surfaces before and after deployment

Run the existing web tests, lint, and optimized build after updating dependencies. Add or adjust only tests needed for changed framework/tooling behavior, with focused coverage for route rendering, authentication, `/admin`, store/WebSocket handling, and terminal snapshots. In a staged production rollout, back up the current web source/build, deploy the reviewed manifest and lockfile, run `npm ci`, build with the required APAS version, restart `apas-web`, and verify pages plus their `/_next/static` assets, API health, WebSocket attachment, and recent logs.

Passing an audit without a production build is insufficient because Next.js, Sharp, and PostCSS affect runtime/build output. Conversely, application regressions must not be “fixed” by weakening the audit threshold or restoring vulnerable versions.

## Risks / Trade-offs

- **[Next.js patch/minor behavior changes routing or server-component output]** → Keep the upgrade within stable 16.x, update `eslint-config-next` in lockstep, run route/component tests and production asset smoke checks.
- **[Vitest pulls a newer Vite line with Node or plugin incompatibilities]** → Check engine and peer constraints, use the current project Node version, run the complete test suite, and select the lowest compatible patched Vitest/Vite combination.
- **[npm and pnpm resolve different vulnerable transitives]** → Regenerate and audit both graphs independently; use dual narrowly scoped overrides only when parent constraints require them.
- **[A new advisory appears while implementation is in progress]** → Treat the current count as a baseline, not an allowlist, and require zero findings from fresh audits immediately before handoff and deployment.
- **[Native Sharp artifacts fail on the production Linux host]** → Exercise a clean production-host `npm ci` and optimized build before restart; retain the previous source, lockfile, and `.next` build for rollback.
- **[Registry availability makes audits flaky]** → Keep audits outside `prebuild`, record the last successful result, and do not deploy until a fresh online audit succeeds.

## Migration Plan

1. Capture the npm and pnpm audit baselines and map each vulnerable transitive to its direct parent.
2. Update Next.js and matching lint configuration, PostCSS, and Vitest to stable patched versions; regenerate npm and pnpm lockfiles with their declared tool versions.
3. Re-audit both clean graphs. Resolve remaining findings through compatible parent upgrades, using documented narrow overrides only when necessary.
4. Run frozen installs, confirm the manifest/lockfiles remain unchanged, then run lint, the complete web test suite, type/build checks, and focused route/WebSocket/terminal regressions.
5. Record remediation and verification evidence and update contributor and deployment commands to use frozen installs.
6. Back up the deployed web source/build, sync the reviewed web tree, run production `npm ci` and the versioned build, restart `apas-web`, and smoke-test public routes, static assets, API health, WebSockets, and logs.

Rollback restores the backed-up web source, npm lockfile, and build (or rebuilds that exact prior lockfile with `npm ci`) before restarting `apas-web`. A rollback knowingly restores the previous vulnerable graph, so it is an emergency availability measure only and must be followed by a corrected patched deployment.
