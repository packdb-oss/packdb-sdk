# Releasing

Only named release maintainers may publish. Every package remains `publish = false`
during private staging. Repository visibility does not make crates.io private.

## Private candidates

1. Select versions and update this changelog and compatibility policy. Packages may
   share a version initially, but each package/version is an independent immutable
   crates.io identity.
2. Run full checks, coverage, compatibility, and security gates on the exact commit.
3. Inspect `cargo package --list` for every package. Each package must include its
   README and LICENSE and exclude private data and internal-only source.
4. Dispatch `oss-candidate`. Review its source archive, dependency inventory,
   per-package inventories, checksums, and release record. A candidate does not
   authorize a tag, GitHub Release, visibility change, or registry upload.

## Publication

Publication requires a separate reviewed change that enables only an explicit
package allowlist and configures registry ownership. Publish dependencies in this
order: `packdb-otel`, then `packdb-client`. Wait for each
crate to become available, verify its registry checksum, and perform a clean consumer
build before publishing the next. Never overwrite or blindly retry a version.

Use crates.io trusted publishing scoped to this repository, workflow, and protected
`public-release` environment when available. The publish workflow stays disabled
while `OSS_PUBLIC_RELEASE_ENABLED` is not `true`, and its adapter refuses publication
until the launch implementation is reviewed. Do not use `pull_request_target` or
`workflow_run` for publication.

Before public visibility, review reachable history, Actions artifacts and private
release attachments; verify security and conduct contacts; record the release-owner review;
and document registry recovery. Describe capabilities and observed limitations
without maturity or production-quality claims. “Used in production” describes use
only and provides no quality, HA, fitness, support, or commercial guarantee.
