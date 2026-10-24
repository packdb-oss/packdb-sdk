# Compatibility

All packages currently use the 0.2.x version line. Before 1.0, a breaking Rust API
change increments the minor version. Patch releases preserve the documented public
API and serialized response shapes. Release notes identify intentional changes.

## Package contracts

- `packdb-otel`: W3C trace-context and baggage propagation through HTTP headers and
  gRPC metadata. Applications own exporter configuration and routing.
- `packdb-client`: no default features. `sql` enables SQL over HTTP;
  `flight-sql` includes `sql` and enables Flight SQL. `ClientError` variants are
  classified by the `packdb` command library; changes to them require its
  contract review.

Every release gate exercises no-default and all-feature builds. Compatibility gates
also test `packdb-client` separately with `sql` and `flight-sql` so feature
unification cannot hide a broken configuration.

## Errors and transports in 0.3.0

Breaking changes:

- `ClientError` is `#[non_exhaustive]` and adds `Unauthenticated` (HTTP `401`,
  gRPC `Unauthenticated`) and `PermissionDenied` (HTTP `403`, gRPC
  `PermissionDenied`). Both were `ConnectionError` before. Flight SQL errors keep
  their gRPC meaning: `InvalidArgument` is `SqlError`, `Unavailable` is
  `ConnectionError`, and other codes are `ProtocolError`.
- `SqlClientConfig::http_only` is replaced by `transport: SqlTransport`
  (`Auto`, `Http`, `FlightSql`); `http_only: true` becomes `SqlTransport::Http`,
  and `http_only: false` becomes `SqlTransport::Auto`.
- `SqlTransport::Auto` no longer falls back to HTTP on every Flight SQL failure.
  It falls back once, only when the Flight SQL connection was refused, the host or
  network was unreachable, or the connect attempt timed out. A failure after the
  query may have been sent is returned, including a stream that fails part-way.
- `ClientError::exit_code` is deprecated; exit codes belong to the command that
  reports the error.

Additions: `SqlClient::execute_query_rows_with_transport`, `RowsAnswer` and
`TransportUsed` report which transport answered and why `Auto` fell back. On
failure it returns `QueryFailure`, which keeps the failing transport and any
fallback reason; it converts into `ClientError`.

Additions without a feature: `QueryApiClient` and `QueryApiConfig` query the
PromQL, LogQL and TraceQL HTTP APIs and return `QueryResponse<serde_json::Value>`;
`LogOptions` and `Direction` carry LogQL's limit and direction. Its failures add
`ClientError::QueryError` (a `400`, or a trace search's `422` for a query the server
does not support) and `ClientError::NotFound` (a trace lookup's `404` whose body is
the server's `{"error": "trace not found: <id>"}` for the requested ID, with no
partial causes); `401` and `403` are `Unauthenticated` and `PermissionDenied`, and
other statuses, including any other `404`, are `ProtocolError` with the server's
message.

An empty `partial_causes` list means the answering transport reported no partial
causes. It is not, by itself, an independent statement that the result is
complete.

## Credential diagnostics in 0.2.3

Debug output redacts credentials, including through AuthConfig and its provider.
The request headers and credential precedence are unchanged. Debug text is not a
wire-format compatibility contract.

## Query completeness in 0.2.2

`QueryResponse<T>` and the completeness-preserving SQL methods are additive.
Existing row-only and formatted-output methods retain their signatures and error
classification. The new methods report causes from the answering HTTP response
or Flight DoGet initial metadata; absent carriers mean complete, and present
empty or unreadable carriers remain partial. Failed Flight attempts do not add
causes to a successful HTTP fallback response.

## Exclusions

The SDK transports credentials supplied by its caller. It does not ship server
lifecycle/build primitives or duplicate Loki/Prometheus response DTOs, issue or
validate credentials, resolve organizations or tenants, call the control plane,
enforce quotas, or perform cell placement and routing. Those are not compatibility
promises of these packages.

The MSRV is Rust 1.94.1. Raising it is a documented release change. Support is
community best effort; this compatibility statement makes no quality, availability,
HA, fitness, or commercial guarantee.
