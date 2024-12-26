# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.13.3] - 2024-12-26

### Fixed

- Require safety documentation for unsafe blocks.
- Fixed [FromRequest](https://docs.rs/picoserve/0.13.3/picoserve/extract/trait.FromRequest.html) for `alloc::vec::Vec`, and by extension, `alloc::string::String`.

### Changes

- [`picoserve::response::chunked::ChunkWriter::write_chunk`](https://docs.rs/picoserve/0.13.3/picoserve/response/chunked/struct.ChunkWriter.html) no longer flushes the socket.
- Removed workaround for embassy-net TcpSocket::flush() never finishing due to upstream bugfix.
- Tidied Content implementation of containers (e.g. Vec and String).
- `impl<C: Content> IntoResponse for C`

## [0.13.2] - 2024-12-10

### Fixed

- Fixed `--inline-threshold` deprecation warning in `embassy` examples.

### Added

- Created an embassy example `app_with_props` to demonstrate building an app with build properties.

## [0.13.1] - 2024-12-06

### Fixed

- Fixed documentation URLs

## [0.13.0] - 2024-12-06

### Breaking

- Updated to embassy-net 0.5, which has a breaking change.

### Added

- Since Rust 1.81, the TAIT (Trait In Type Aliases) usage is disallowed. Added helper traits [`picoserve::AppBuilder`](https://docs.rs/picoserve/0.13.0/picoserve/trait.AppBuilder.html) and [`picoserve::AppWithStateBuilder`](https://docs.rs/picoserve/0.13.0/picoserve/trait.AppWithStateBuilder.html) to simplify creating a static app using ITIAT (Impl Trait In Associated Type).

### Changed

- Examples use ITIAT (Impl Trait In Associated Type) instead of TAIT (Trait In Type Aliases).

## [0.12.3] - 2024-11-24

### Fixed

- Fixed return type of [`picoserve::routing::Router::nest`](https://docs.rs/picoserve/0.12.3/picoserve/routing/struct.Router.html#method.nest).

### Added

- Added example demonstrating nesting [`Router`s](https://docs.rs/picoserve/0.12.3/picoserve/routing/struct.Router.html).
- Added support for "PUT" and "DELETE" HTTP methods.

### Changed
- `alloc::string::String` now implements IntoResponse (previously `std::string::String` did).

## [0.12.2] - 2024-08-14

### Fixed

- `Debug2Format` implements "core::fmt::Debug"

## [0.12.1] - 2024-08-07

### Added

- Added support for deserializing JSON values using the [Json](https://docs.rs/picoserve/0.12.1/picoserve/extract/struct.Json.html) Extractor.

## [0.12.0] - 2024-07-22

### Breaking

- Deprecated and removed `ShutdownMethod`, `Config::shutdown_connection_on_close`, `Config::abort_connection_on_close`, `Socket::abort`.
- `<embassy_net::tcp::TcpSocket as picoserve::io::Socket>::shutdown` now aborts the flush once the socket state is `Closed`.
- [`File`](https://docs.rs/picoserve/0.12.0/picoserve/response/fs/struct.File.html) no longer implements [`IntoResponse`](https://docs.rs/picoserve/0.12.0/picoserve/response/trait.IntoResponse.html).

### Fixed

- No longer flushing forever if the already closed while an `embassy_net::tcp::TcpSocket` socket is shutting down.
- Fixed bug in `<ETag as PartialEq<[u8]>>::eq`.

### Changed

- Changed where the socket is flushed, avoid double-flushes.

### Added

- [Layers](https://docs.rs/picoserve/0.12.0/picoserve/routing/trait.Layer.html) can take ownership of requests, allowing them to:
  - Route requests to a different Router.
  - Not call the next layer, but return a response.
- Implemented [Chunked](https://docs.rs/picoserve/0.12.0/picoserve/response/chunked/struct.ChunkedResponse.html) Transfer Encoding.
- Added [CustomResponse](https://docs.rs/picoserve/0.12.0/picoserve/response/custom/index.html), allowing for responses with a body that doesn’t match a regular HTTP response.

## [0.11.1] - 2024-06-06

### Fixed

- `FrameWriter` now implements flush, which just flushes its internal writer. Resolves potential future correctness issues.
- Removed unnessessary `unsafe` code in `FormatBuffer`

## [0.11.0] - 2024-05-30

### Breaking

- Disabled decoding escape sequences in Headers, as was incorrectly implemented, and in fact escape sequences are deprecated.

### Fixed

- Headers are `[u8]` not `str`, to allow header names and values that are not UTF-8.
- Correctly handling zero-length Web Socket payloads.

### Added

- SSE Data can now contain newlines.
- SSE Data can now be a `core::fmt::Arguments`, as produced by `format_args!`.
- Web Sockets can now send `core::fmt::Arguments` as a series of text frames, allowing for sending formatted text messages.

## [0.10.3] - 2024-05-06

### Added

- Added [Router::from_service](https://docs.rs/picoserve/0.10.3/picoserve/routing/struct.Router.html#method.from_service) which creates a Router from a [PathRouterService](https://docs.rs/picoserve/0.10.3/picoserve/routing/trait.PathRouterService.html), allowing a custom fallback service should routing fail to find a suitable handler or service. Added `routing_fallback` example to demonstrate this.

## [0.10.2] - 2024-03-19

### Fixed

- Added double quote to the list of allowed characters in headers.

## [0.10.1] - 2024-03-17

### Changed

- Optionally abort the connection instead of performing a graceful shutdown after handling all requests.

### Fixed

- Fixed compilation errors when enabling "defmt" feature.

## [0.10.0] - 2024-03-10

### Breaking

- Several public types have changed name to improve name consistency.
- Sealed [RequestHandler](https://docs.rs/picoserve/0.10.0/picoserve/routing/trait.RequestHandler.html), [MethodHandler]((https://docs.rs/picoserve/0.10.0/picoserve/routing/trait.MethodHandler.html)), and [PathRouter](https://docs.rs/picoserve/0.10.0/picoserve/routing/trait.PathRouter.html), and added [RequestHandlerService](https://docs.rs/picoserve/0.10.0/picoserve/routing/trait.RequestHandlerService.html) and [PathRouterService](https://docs.rs/picoserve/0.10.0/picoserve/routing/trait.PathRouterService.html) which have better ergonomics.
  - A service with no path parameters now has path parameters of `()` not `NoPathParameters`.
  - A service with a single path parameter now has path parameters of `(T,)`, not `OnePathParameter`.
  - A service with multiple path parameters now has a tuple of path parameters, not `ManyPathParameters`.
- Moved Status Code constants to inside [StatusCode](https://docs.rs/picoserve/0.10.0/picoserve/response/status/struct.StatusCode.html).
- [FromRequest](https://docs.rs/picoserve/0.10.0/picoserve/extract/trait.FromRequest.html) and [FromRequestParts](https://docs.rs/picoserve/0.10.0/picoserve/extract/trait.FromRequestParts.html) are now generic over the lifetime of the request, allowing them to borrow from the request.
- Logging using the `log` crate is only enabled if the `log` feature is enabled

### Added

- Added [from_request](https://docs.rs/picoserve/0.10.0/picoserve/macro.from_request.html) and [from_request_parts](https://docs.rs/picoserve/0.10.0/picoserve/macro.from_request.html) as convenience for [PathRouters](https://docs.rs/picoserve/0.10.0/picoserve/routing/trait.PathRouter.html), and added [RequestHandlerServices](https://docs.rs/picoserve/0.10.0/picoserve/routing/trait.RequestHandlerService.html) which borrow from Requests, which is now permitted.
- Added support for percent-encoding in headers.
- If the `defmt` feature is enabled:
  - All public type which implement `Debug` also implement `defmt::Format`
  - Logging is done using `defmt`

## [0.9.1] - 2024-02-12

### Added

- [`File`](https://docs.rs/picoserve/0.9.1/picoserve/response/fs/struct.File.html) now has optional headers, allowed for fixed headers to be declared per file
- [`Content`](https://docs.rs/picoserve/0.9.1/picoserve/response/trait.Content.html) is implemented for `Vec<u8>` and `String` behind the `alloc` feature

## [0.9.0] - 2024-02-12

### Breaking

- Request bodies are no longer automatically read, but must be read.
- [`request::Reader`](https://docs.rs/picoserve/0.8.1/picoserve/request/struct.Reader.html) is no longer public.
- Connection must be given an [`UpgradeToken`](https://docs.rs/picoserve/0.9.0/picoserve/extract/struct.UpgradeToken.html) when upgraded.
- [`ResponseWriter`](https://docs.rs/picoserve/0.9.0/picoserve/response/trait.ResponseWriter.html) must be given a [`Connection`](https://docs.rs/picoserve/0.9.0/picoserve/response/struct.Connection.html) when writing the response.

### Changes

- Request Bodies can not be either read into the internal buffer (as previously), or converted into a [`RequestBodyReader`](https://docs.rs/picoserve/0.9.0/picoserve/response/struct.RequestBodyReader.html), which implements Read.

### Added

- Added several unit tests around routing and reading requests.

## [0.8.1] - 2024-02-05

### Changes

- Fixed newline in WebSocketKeyHeaderMissing message.

## [0.8.0] - 2024-02-05

### Breaking

- [`serve`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve.html) and [`serve_with_state`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve_with_state.html) now take a socket rather than a reader and writer.

### Changes

- The socket is now shut down after it has finished handling requests

### Added

- Added support for [`embassy`](https://github.com/embassy-rs/embassy) with the `embassy` feature.
  - No need to declare and pass in a timer, used Embassy timers
  - Pass a [`TcpSocket`](https://docs.rs/embassy-net/0.4.0/embassy_net/tcp/struct.TcpSocket.html) to [`serve`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve.html) and [`serve_with_state`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve_with_state.html)
  - Added more examples which use embassy

## [0.7.2] - 2024-02-05

### Changes

- Using const_sha from crates.io (rather than copied into this repository) as it now has no_std support

## [0.7.1] - 2024-01-24

### Changes

- [Config::new](https://docs.rs/picoserve/0.7.1/picoserve/struct.Config.html#method.new) is now const

## [0.7.0] - 2024-01-20

### Fixed

- The "Connection" header is no longer sent in duplicate if the handler has already sent it

### Changes

- The handling of the "Connection" header in the request has changed:
  - If `picoserve` has been configured to always close the connection after responding, set the "Connection" header to "close".
    - This is the default, overide by calling `keep_connection_alive` on [Config](https://docs.rs/picoserve/0.6.0/picoserve/struct.Config.html).
  - If not:
    - If the "Connection" header is missing, then check the HTTP version. If the HTTP version is equal to "HTTP/1.1", then keep the connection alive, else close the connection.
    - If the "Connection" header is "close", close the connection.
    - If the "Connection" header is a comma separated list and one of the entries is "upgrade", such as a websocket upgrade, close the connection after handling the response. Either the handler will handle the upgrade, setting the "Connection" header, in which case it will not also be automatically sent, or something has gone wrong, and the connection should be closed. Also, an upgraded connection, which is thus no longer HTTP, should be closed after completion, not reused.
      - Note that the connection is closed after the "response" has been sent. In the case of websockets, sending the "response" includes sending messages to the client and also parsing incoming messages, so this is fine.
- The title of the web_sockets example has been changed from "Server-Sent Events" to "Websockets"
- Frame, Control, Data, and Message in [`response::ws`](https://docs.rs/picoserve/0.7.0/picoserve/response/ws/index.html) now implement Debug

## [0.6.0] - 2024-01-02

### Breaking

- Changed [Config](https://docs.rs/picoserve/0.6.0/picoserve/struct.Config.html) structure
  - Moved timeouts to separate structure
  - Added `connection` field, describing behaviour after the response has been sent
- Defaults to closing connection after the response has been sent
  - To preserve previous behaviour, which requires multiple concurrent sockets accepting connections, call `keep_connection_alive` on [Config](https://docs.rs/picoserve/0.6.0/picoserve/struct.Config.html)

### Changes

- Allow configuration of behaviour after the response has been sent, i.e. should the TCP connection be closed or kept alive?
  - If the request does not include the "Connection" header or it is set to "close", the response includes a header of "Connection: close", and the connection is closed after the response has been sent. Otherwise, the response includes a header of "Connection: keep-alive", and the connection is kept alive after the response has been sent, allowing additional requests to be made on the same TCP connection.

## [0.5.0] - 2024-01-02

picoserve now runs on stable!

### Breaking

- No longer using the `async_fn_in_trait` feature
- `hello_world_embassy` example uses newer version of `embassy` (and still requires nightly rust due to `embassy` using feature `type_alias_impl_trait`)

## [0.4.1] - 2023-12-23

### Fixed

- Fixed JSON serialization for empty objects and arrays

## [0.4.0] - 2023-12-23

### Breaking

- Parsing [Query](https://docs.rs/picoserve/0.4.0/picoserve/extract/struct.Query.html) and [Form](https://docs.rs/picoserve/0.4.0/picoserve/extract/struct.Form.html) now ignores blank space between two `&` characters
  - This allows urls which end in `?` but have no query, and urls for which there's a `&` after the query

## [0.3.0] - 2023-12-11

### Breaking

- [Config](https://docs.rs/picoserve/0.3.0/picoserve/struct.Config.html) now has a field `write_timeout`
- [Error](https://docs.rs/picoserve/0.3.0/picoserve/enum.Error.html) has an extra variant `WriteTimeout`

### Changes

- If `write_timeout` is `Some(timeout)` in [Config](https://docs.rs/picoserve/0.3.0/picoserve/struct.Config.html), writing data to the client will fail with `Error::WriteTimeout` if the write times out

## [0.2.3] - 2023-11-18

- Improved documentation

## [0.2.2] - 2023-11-03

### Added

- Added `into_response(self)` method to [response::fs::File](https://docs.rs/picoserve/0.2.2/picoserve/response/fs/struct.File.html), [response::json::Json](https://docs.rs/picoserve/0.2.2/picoserve/response/json/struct.Json.html), and [response::sse::EventStream](https://docs.rs/picoserve/0.2.2/picoserve/response/sse/struct.EventStream.html), converting them into a [response::Response](https://docs.rs/picoserve/0.2.2/picoserve/response/struct.Response.html)
- Added documentation to [response::Response](https://docs.rs/picoserve/0.2.2/picoserve/response/struct.Response.html)

## [0.2.1] - 2023-10-29

### Fixed

- Fixed documentation to match changes to rust version `nightly-2023-10-02`, changed in 0.2.0

## [0.2.0] - 2023-10-29

### Breaking

- Updated `embedded-io-async` to 0.6.0
- Many methods return `SomeErrorType` not `picoserve::io::WriteAllError<SomeErrorType>`
- Now using rust `nightly-2023-10-02`

## [0.1.2] - 2023-09-01

### Added

- Added hello_world_embassy example to README

## [0.1.1] - 2023-09-01

- First Release