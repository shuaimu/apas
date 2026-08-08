# Web Dependency Security Specification

## Purpose

Defines the security and reproducibility guarantees for dependencies used to build, test, and run the APAS web application before a release is deployed.

## Requirements

### Requirement: Supported web dependency graphs contain no known vulnerabilities
Every checked-in dependency graph supported for installing the APAS web application SHALL resolve without vulnerabilities reported by its package manager's current advisory database. This requirement SHALL cover production and development dependencies at every severity level.

#### Scenario: npm release graph is audited
- **WHEN** the npm dependency graph represented by the reviewed manifest and lockfile is installed and audited
- **THEN** the audit reports zero known vulnerabilities across production and development dependencies

#### Scenario: pnpm contributor graph is audited
- **WHEN** the pnpm dependency graph represented by the reviewed manifest and lockfile is installed and audited
- **THEN** the audit reports zero known vulnerabilities across production and development dependencies

#### Scenario: a new advisory affects the resolved graph
- **WHEN** either supported audit reports a vulnerability at any severity
- **THEN** dependency-security verification fails
- **AND** the web release is not considered ready until the graph is patched or a separately reviewed security exception changes this requirement

### Requirement: Web dependency installations are reproducible
The repository SHALL keep the web manifest and every supported lockfile synchronized. Production and verification installs MUST consume a checked-in lockfile in frozen mode and MUST fail rather than silently rewriting dependency resolution.

#### Scenario: production dependencies are installed
- **WHEN** the production web application is installed for a build or deployment
- **THEN** npm installs the exact graph recorded in the reviewed npm lockfile
- **AND** the install leaves the manifest and lockfile unchanged

#### Scenario: contributor dependencies are installed with pnpm
- **WHEN** a contributor performs the documented frozen pnpm install
- **THEN** pnpm installs the exact graph recorded in the reviewed pnpm lockfile
- **AND** the install leaves the manifest and lockfile unchanged

#### Scenario: a manifest and lockfile disagree
- **WHEN** either supported frozen install detects that its lockfile does not match the manifest
- **THEN** installation fails with an actionable dependency-resolution error
- **AND** no build or deployment proceeds from an implicitly regenerated graph

### Requirement: Dependency remediation preserves web behavior
Upgrading or constraining web dependencies SHALL preserve APAS's existing public routes, authentication flows, administrator controls, WebSocket operation, and terminal presentation unless a separate product change explicitly modifies them.

#### Scenario: remediated web verification runs
- **WHEN** the patched dependency graphs pass their security audits
- **THEN** web lint, automated tests, type checking, and the optimized production build also pass

#### Scenario: remediated build is smoke-tested
- **WHEN** the patched production build is started behind the APAS edge configuration
- **THEN** the root, login, machines, share, and administrator pages load successfully
- **AND** their referenced static assets are retrievable
- **AND** authentication, API health, WebSocket attachment, and terminal rendering remain operational

### Requirement: Dependency-security evidence is reviewable
The dependency update SHALL make the vulnerable baseline, resolved remediations, audit results, compatibility verification, and any exceptional transitive constraints reviewable without including credentials or private registry tokens.

#### Scenario: reviewer evaluates the dependency update
- **WHEN** a reviewer inspects the completed change
- **THEN** the review material identifies the direct and transitive packages that were remediated
- **AND** records clean npm and pnpm audit summaries and the web regression results
- **AND** explains every package override or reports that none was required
