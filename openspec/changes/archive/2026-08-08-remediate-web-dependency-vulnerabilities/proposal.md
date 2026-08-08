## Why

The production web install currently reports 15 known dependency vulnerabilities (1 critical, 12 high, 1 moderate, and 1 low), including direct advisories in Next.js, PostCSS, and Vitest. APAS needs patched, reproducible npm and pnpm dependency graphs so production and contributor installs do not retain known fixable vulnerabilities or silently diverge.

## What Changes

- Upgrade the direct web dependencies that anchor vulnerable runtime and development subgraphs, including Next.js, PostCSS, and Vitest, to compatible patched releases.
- Refresh vulnerable transitive dependencies such as Sharp, Vite, Rollup, ws, minimatch, picomatch, brace-expansion, js-yaml, Nano ID, flatted, AJV, and Babel through parent upgrades or narrowly scoped overrides where normal resolution cannot select a patched release.
- Keep `package-lock.json` and `pnpm-lock.yaml` synchronized with `package.json`, because production installs with npm while the repository's package metadata and Makefile advertise pnpm to contributors.
- Add repeatable dependency-security verification that fails when either supported install path resolves a known vulnerability at the configured audit threshold.
- Run the complete web lint, test, type/build, and production smoke suite after the dependency refresh, with focused regression coverage for Next.js routing, WebSocket connectivity, authentication, terminal rendering, and the `/admin` page.
- Document the dependency update and deployment/rollback checks; no application API or user workflow is intentionally changed.

## Capabilities

### New Capabilities

- `web-dependency-security`: Defines secure, reproducible web dependency resolution and the audit and regression gates required before deployment.

### Modified Capabilities

None. There are no existing main OpenSpec capabilities in this planning root to modify.

## Impact

- Affects `packages/web/package.json`, `package-lock.json`, `pnpm-lock.yaml`, dependency verification scripts, and contributor/release documentation.
- Updates the deployed Next.js runtime and build/test toolchain, including transitive native Sharp packages used by Next.js image handling.
- May require small compatibility adjustments in web source or tests if patched framework/tooling behavior changed, but must preserve existing HTTP routes, authentication, WebSocket behavior, terminal panes, and administrator controls.
- Production deployment continues to use npm and must be built from the reviewed lockfile; pnpm remains a supported contributor path and must resolve an equivalently vulnerability-free graph.
