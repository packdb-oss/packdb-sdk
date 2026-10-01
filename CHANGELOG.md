# Changelog

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
