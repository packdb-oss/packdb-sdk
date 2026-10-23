# Changelog

## Unreleased

- Breaking: `ClientError` is `#[non_exhaustive]` and distinguishes
  `Unauthenticated` (401) and `PermissionDenied` (403), which were
  `ConnectionError`. Flight SQL errors keep their gRPC status meaning.
- Breaking: `SqlClientConfig::http_only` is replaced by `transport:
  SqlTransport` (`Auto`, `Http`, `FlightSql`). An explicit transport never falls
  back. `Auto` falls back to HTTP only when the Flight SQL connection could not be
  opened before any request was sent, not after a failure part-way through a
  query.
- Add `SqlClient::execute_query_rows_with_transport`, returning `RowsAnswer`
  with the transport that answered and any fallback reason.
- Deprecate `ClientError::exit_code`.

## 0.2.4

- Replace the LICENSE files (repository root, `packdb-client` and `packdb-otel`)
  with the verbatim Apache License 2.0 text. Earlier releases shipped an altered
  text: the Trademarks clause (section 6) was missing, sections 7 to 9 were
  renumbered 6 to 8, the copyright grant wording differed, and section 9 was
  retitled. The crates' licensing (Apache-2.0, as declared in `Cargo.toml` and
  the source headers) is unchanged. Earlier release archives keep their original
  text; this release supersedes them.
- Update the locked `rustls-webpki` to 0.103.16 and `webpki-root-certs` to
  1.0.10, resolving RUSTSEC-2026-0382 and RUSTSEC-2026-0383. This covers this
  repository's own builds and release archives only: Cargo does not apply a
  library's lockfile to its dependents. Applications that depend on these crates
  must update their own `Cargo.lock` to `rustls-webpki` 0.103.16 or later and
  `webpki-root-certs` 1.0.10 or later, for example with
  `cargo update -p rustls-webpki -p webpki-root-certs`.
- No public API or wire-format changes.

## 0.2.3

- Redact API keys, bearer tokens, basic-auth credentials and nested credential
  providers from Debug output. Request authentication behavior is unchanged.
- Update the locked Rustls dependency to 0.23.46, resolving
  RUSTSEC-2026-0346, RUSTSEC-2026-0347, RUSTSEC-2026-0348 and RUSTSEC-2026-0349.
- Support single-maintainer public operation with recorded reviews and passing
  checks. Pin candidate qualification to Ubuntu 24.04 for Python compatibility.

## 0.2.2

- Add `QueryResponse` and `SqlClient::execute_query_rows_with_completeness` to
  retain `x-packdb-partial-causes` from HTTP headers or Flight SQL DoGet initial
  metadata. Empty and malformed carriers fail closed. HTTP fallback retains
  only the answering response's causes. Existing row-only methods are unchanged.
- Replace the yanked `yoke-derive 0.8.3` lockfile entry with `0.8.4`.
- Update pinned CI actions, add Dependabot and scheduled action-pin checks, and
  record this repository as the canonical source of the SDK.

## 0.2.1

- `packdb-client`: a query the server rejects over HTTP (`400` with a
  `Query error:` or `Parse error:` body) is now `ClientError::SqlError`, and
  `401`/`403` are `ClientError::ConnectionError`. Every other non-2xx response,
  including every `5xx`, remains `ClientError::ProtocolError`. 0.2.0 reported
  all of them as `ProtocolError`.

## 0.2.0

- Prepared `packdb-otel` and `packdb-client` as a standalone Apache-2.0 workspace.
- Excluded unreferenced server primitives and duplicate protocol response DTOs.
- Removed control-plane, tenancy, authentication issuance, placement, routing,
  quota, reliability administration, and other commercial service contracts from
  the public dependency closure.
- Added reproducible export provenance and private candidate workflows.
