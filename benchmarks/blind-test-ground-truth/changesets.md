# changesets
> https://github.com/changesets/changesets | TypeScript | ts monorepo release tool | ~60k LOC

## architecture
- packages/cli — the CLI: changeset init/add/version/publish (packages/cli/)
- packages/cli/src/cli.ts — the CLI entry: cli = cac("changeset") (packages/cli/src/cli.ts)
- packages/cli/src/commands — the command implementations: add, version, publish, status, init, pre, pack (packages/cli/src/commands/)
- packages/apply-release-plan — version bumping: applyReleasePlan (packages/apply-release-plan/)
- packages/assemble-release-plan — release plan assembly: assembleReleasePlan (packages/assemble-release-plan/)
- packages/get-release-plan — the plan getter (packages/get-release-plan/)
- packages/get-dependents-graph — the dependents graph (packages/get-dependents-graph/)
- packages/get-version-range-type — the range types (packages/get-version-range-type/)
- packages/get-github-info — GitHub info fetching (packages/get-github-info/)
- packages/changelog-git — the git changelog (packages/changelog-git/)
- packages/changelog-github — the GitHub changelog (packages/changelog-github/)
- packages/read — changeset reading (packages/read/)
- packages/write — changeset writing (packages/write/)
- packages/parse — changeset parsing (packages/parse/)
- packages/config — the config schema (packages/config/)
- packages/types — the shared types (packages/types/)
- packages/git — git utilities (packages/git/)
- packages/errors — the error types (packages/errors/)
- packages/logger — the logger (packages/logger/)
- packages/pre — the pre-release mode (packages/pre/)

## entrypoints
- changeset init — initialize changesets in a repo
- changeset add — add a new changeset
- changeset version — version packages and write changelogs
- changeset publish — publish to npm and tag
- changeset status — show existing changesets
- changeset pre — enter pre-release mode
- changeset pack — pack publishable packages
- changeset git-tag — create git tags
- changeset publish-plan — show publish-ready packages
- cli — the cac CLI entry
- applyReleasePlan — the plan application entry
- assembleReleasePlan — the plan assembly entry
- readChangesets — the changeset reader
- getReleasePlan — the plan getter
- getDependentsGraph — the dependents graph entry

## behavior
- changeset add -> createChangeset -> write md (commands/add/)
- changeset version -> getReleasePlan -> applyReleasePlan -> version bumps (commands/version/)
- changeset publish -> npm publish -> git tag (commands/publish/)
- assembleReleasePlan -> dependents graph -> version types (assemble-release-plan/)
- applyReleasePlan -> package.json updates -> changelog (apply-release-plan/)
- read -> parse -> changesets list (read/)
- getDependentsGraph -> package graph (get-dependents-graph/)
- changeset status -> list changesets (commands/status/)
- pre enter -> pre mode state (commands/pre/)

## state_authority
- ReleasePlan — the release plan state (assemble-release-plan/)
- Config — the changesets config state (config/)
- Changeset — the changeset state: summary, releases (types/)
- PreState — the pre-release state (pre/)
- DependentsGraph — the dependents graph state (get-dependents-graph/)
- VersionType — the version type state (get-version-range-type/)
- package.json — the package version state
- git tag — the tag state

## contracts
- changeset add --empty — empty changeset contract
- changeset add --patch --minor — bump type contract
- changeset version --ignore pkg — version contract
- changeset publish --tag beta — publish contract
- changeset status --since main — status contract
- changeset pre enter beta — pre mode contract
- changeset init — init contract
- changeset pack — pack contract
- changeset git-tag — git-tag contract
- .changeset/config.json — config contract
- .changeset/<name>.md — changeset file contract
- "patch" — patch bump contract
- "minor" — minor bump contract
- "major" — major bump contract
- "none" — no-bump contract
- npm publish — publish contract

## landmarks
- cli — the CLI entry (packages/cli/src/cli.ts)
- applyReleasePlan — the plan applier (packages/apply-release-plan/)
- assembleReleasePlan — the plan assembler (packages/assemble-release-plan/)
- getReleasePlan — the plan getter (packages/get-release-plan/)
- getDependentsGraph — the graph builder (packages/get-dependents-graph/)
- readChangesets — the reader (packages/read/)
- Changeset — the changeset type (packages/types/)
- ReleasePlan — the release plan type (packages/types/)
- Config — the config type (packages/config/)
- createChangeset — the add command (packages/cli/src/commands/add/createChangeset.ts)

## tests
- packages/cli/src/commands/version/version.test.ts — version tests
- packages/cli/src/cli.test.ts — CLI tests
- packages/apply-release-plan/src/index.test.ts — plan tests
- packages/assemble-release-plan/src/index.test.ts — assembly tests
- packages/read/src/index.test.ts — read tests
- packages/parse/src/index.test.ts — parse tests
- packages/cli/src/commands/add/__tests__/ — add command tests
- packages/cli/src/commands/__tests__/ — command tests
- packages/changelog-git/src/index.test.ts — changelog tests
