# Contributing

## Versions

Use [Semantic Versioning](https://semver.org/). The application version in
`Cargo.toml` is the source of truth; keep its package entry in `Cargo.lock`
in sync when changing it.

The initial experimental version is `0.1.0`. During `0.x` development:

- Patch versions contain backward-compatible fixes.
- Minor versions contain features or breaking changes. Document any breaking
  change and migration steps explicitly.
- Move to `1.0.0` when the supported interfaces and compatibility promises are
  stable. After that, breaking changes require a major version.

Supported interfaces include document formats, command-line arguments, and
documented backend integration. A UI layout adjustment alone is not necessarily
a breaking change.

Do not bump the version on every commit. Update it deliberately for a release;
use `vX.Y.Z` release tags only when publishing that release.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/):

```text
type(optional-scope): concise description
```

Use `feat` for features, `fix` for bug fixes, and `docs`, `test`, `refactor`,
`build`, `ci`, or `chore` as appropriate. Mark breaking changes with `!` and
explain them in a `BREAKING CHANGE:` footer.

Examples:

```text
feat(heap): add fragmentation map inspection
fix(recording): retain the current session after failed imports
docs: explain west module setup
```

Work on a separate branch and open a pull request rather than pushing directly
to `main`. Before requesting review, run the checks in README.md and report
any manual visual or platform checks that were not performed.
