# Changelog

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
