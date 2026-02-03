# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Breaking

- Routing types which have `PathParameters` use tuples rather than one of `NoPathParameters`, `OnePathParameter`, `ManyPathParameters`.
- [`Router`](https://docs.rs/picoserve/latest/picoserve/routing/struct.Router.html) has a different number of type parameters.
- Updated `picoserve_derive` to version 0.1.4.
- The method of [`PathRouterService`](https://docs.rs/picoserve/latest/picoserve/routing/trait.PathRouterService.html) is corrected, changing from `call_request_handler_service` to `call_path_router_service`.
- Updated to `embedded-io-async` 0.7, `embassy-net` 0.8.0 and `embassy-time` 0.5.0, which have breaking changes.
- [`Timer`](https://docs.rs/picoserve/latest/picoserve/time/trait.Timer.html) must use [`Duration`](https://docs.rs/picoserve/latest/picoserve/time/struct.Duration.html) instead of its own time. If the `embassy` feature is enabled, `Duration` becomes an alias of [`embassy_time::Duration`](https://docs.rs/embassy-time/latest/embassy_time/struct.Duration.html).
- The [`Rejection](https://docs.rs/picoserve/latest/picoserve/extract/trait.FromRequest.html#associatedtype.Rejection) type for `&mut [u8]`, `&[u8]`, `alloc::vec::Vec<u8>`, and `alloc::borrow::Cow<'r, [u8]>` is now [`ReadAllBodyError`](https://docs.rs/picoserve/latest/picoserve/request/enum.ReadAllBodyError.html).
- Timeouts are no longer optional.

### Fixed
- The [`Debug`](https://doc.rust-lang.org/core/fmt/trait.Debug.html) implementation of [`HeaderName`](https://docs.rs/picoserve/latest/picoserve/request/struct.HeaderName.html) and [`HeaderValue`](https://docs.rs/picoserve/latest/picoserve/request/struct.HeaderValue.html) includes the surrounding double quotes.
- The `read_request` timeout applies to reading the request body, not just the request head.
- [`Config`](https://docs.rs/picoserve/latest/picoserve/struct.Config.html) can now be `const` or `static`.

### Changed
- If a request handler doesn't read the entire request body and there is data waiting to be read from the socket, the connection is closed, avoiding needlessly reading and discarding a potentially large request body.

### Added
- Added [`MethodHandlerService`](https://docs.rs/picoserve/latest/picoserve/routing/trait.MethodHandlerService.html) and [`Router::route_service`](https://docs.rs/picoserve/latest/picoserve/routing/struct.Router.html#method.route_service).
- Added support for the `PATCH` and `TRACE` HTTP methods.
- Added `const_new` methods to [`Config`](https://docs.rs/picoserve/latest/picoserve/struct.Config.html), [`Timeouts`]([`Config`](https://docs.rs/picoserve/latest/picoserve/struct.Timeouts.html)), and [`KeepAlive`](https://docs.rs/picoserve/latest/picoserve/enum.KeepAlive.html).
- `heapless::Vec<u8, N>` and `heapless::String<N>` implements [`FromRequest`](https://docs.rs/picoserve/latest/picoserve/extract/trait.FromRequest.html).

## [0.17.1] - 2025-11-07

### Fixed
- Fixed attributes for docs.rs build. feature `doc_auto_cfg` has been replaced with `doc_cfg`

## [0.17.0] - 2025-11-07

### Breaking

- Changed MSRV to 1.85.
- Request Handlers closures must be async closures, not just return futures.
- Removed trivial `IntoFuture` implementations for response types.
- JSON and Websocket functionality are behind the `json` and `ws` features respectively.
- ['Json'](https://docs.rs/picoserve/0.17.0/picoserve/extract/struct.Json.html) no longer has a constant of the string unescape buffer size, use ['JsonWithUnescapeBufferSize'](https://docs.rs/picoserve/0.17.0/picoserve/extract/json/struct.JsonWithUnescapeBufferSize.html) to specify this.
- Changed [`UpgradedWebSocket<UnspecifiedProtocol, C>`](https://docs.rs/picoserve/0.17.0/picoserve/response/ws/struct.UpgradedWebSocket.html) to `UpgradedWebSocket<UnspecifiedProtocol, CallbackNotUsingState<C>>`.
- Removed `serve`, `serve_with_state`, `listen_and_serve`, and `listen_and_serve_with_state`. See the migration guide below.
- [`Timer`](https://docs.rs/picoserve/0.17.0/picoserve/time/trait.Timer.html) and [`Socket`](https://docs.rs/picoserve/0.17.0/picoserve/io/trait.Socket.html) have a generic parameter of the runtime that they use. If you have a custom `Timer` or `Socket` running on `embassy`, use `picoserve::EmbassyRuntime` (which is hidden in docs) as the parameter.
- [`Timer::run_with_timeout`](https://docs.rs/picoserve/0.17.0/picoserve/time/trait.Timer.html#tymethod.run_with_timeout) now takes `&self`, not `&mut self`.
- [`SocketRx::next_frame`](https://docs.rs/picoserve/0.17.0/picoserve/response/ws/struct.SocketRx.html#method.next_frame) and [`SocketRx::next_message`](https://docs.rs/picoserve/0.17.0/picoserve/response/ws/struct.SocketRx.html#method.next_message) take a signal to simplify awaiting multiple sources and help avoiding the case where a read is dropped partway through reading a frame due to the usage of `select!` or similar.

### Fixed

- Fixed [`from_request`](https://docs.rs/picoserve/0.17.0/picoserve/macro.from_request.html) and [`from_request_parts`](https://docs.rs/picoserve/0.17.0/picoserve/macro.from_request_parts.html) macros.
- Fixed lifetime of [`HeaderValue::as_str`](https://docs.rs/picoserve/0.17.0/picoserve/request/struct.HeaderValue.html#method.as_str).
- Fixed routing logic of [`Router::nest`](https://docs.rs/picoserve/0.17.0/picoserve/routing/struct.Router.html#method.nest) and [`Router::nest_service`](https://docs.rs/picoserve/0.17.0/picoserve/routing/struct.Router.html#method.nest_service), where previously a path of `"/path"` was incorrectly routed to a nest with a prefix of `"/path"`, leaving an invalid path of `""`.
- Removed race condition bug in Websockets example.

### Added
- Added support for Websockets which have access to the state with [`WebSocketUpgrade::on_upgrade_using_state`](https://docs.rs/picoserve/0.17.0/picoserve/response/ws/struct.WebSocketUpgrade.html#method.on_upgrade_using_state).
- Added support for the `OPTIONS` HTTP method.
- Added [`Server`](https://docs.rs/picoserve/0.17.0/picoserve/struct.Server.html), a HTTP Server.
- Added support for graceful shutdown of connections using [`Server::with_graceful_shutdown`](https://docs.rs/picoserve/0.17.0/picoserve/struct.Server.html#method.with_graceful_shutdown).
- Added mime-type constants to [`File`](https://docs.rs/picoserve/0.17.0/picoserve/response/fs/struct.File.html).
- Added [`WithStateUpdate`](https://docs.rs/picoserve/0.17.0/picoserve/response/with_state/trait.WithStateUpdate.html) for easily adding state updates to responses.

### Changed
- `embassy` sockets have tcp keepalive and timeout set to 30s and 45s respectively, thus helping prevent broken connections lingering.

### Migration Guide.

There are two new big concepts:

- [`Server`](https://docs.rs/picoserve/0.17.0/picoserve/struct.Server.html), which replaces the `picoserve::serve` and `picoserve::listen_and_serve` functions.
- [`Router::with_state`](https://docs.rs/picoserve/0.17.0/picoserve/routing/struct.Router.html#method.with_state) and [`Router::shared`](https://docs.rs/picoserve/0.17.0/picoserve/routing/struct.Router.html#method.shared), which, together with [`Server`](https://docs.rs/picoserve/0.17.0/picoserve/struct.Server.html) replaces the `picoserve::*_with_state` functions.

Also, handler functions must now be async closures, rather than closures that return a `Future`.

#### Server

`Server` is a HTTP Server, and is able to either serve requests read from a given [`Socket`](https://docs.rs/picoserve/0.17.0/picoserve/io/trait.Socket.html),
or if the `embassy` feature is enabled, listen for incoming connections and serve requests on the connection.

`Server` is designed to be very lightweight, and is typically just a reference to a [`Router`](https://docs.rs/picoserve/0.17.0/picoserve/routing/struct.Router.html). As such a typical usecase will have a single `Router` shared between tasks, but each task has its own `Server`.

#### `Router::with_state`

`Server` only accepts a `Router` with no state (or rather a state of `()`).

A `Router` with a state can be converted into a stateless `Router` using `Router::with_state`,
either passing the state itself if the `Router` should own the state,
or passing in a reference to the state if the `Router` should borrow the state, i.e. the `Router` and state are stored separately.

#### Migration recipies

##### `serve` or `listen_and_serve`

Create a `Server` and call `serve` or `listen_and_serve`.

##### `serve_with_state` or `listen_and_serve_with_state` where the state is created at the same time as the `Router`

Call `with_state` on the `Router` after the routes have been declared, and pass in the state.

If you used `AppWithStateBuilder`, have the state be one of the fields of your builder type, and have it implement `AppBuilder` instead.

The "state" example demonstrates this pattern.

##### `serve_with_state` or `listen_and_serve_with_state` where the state is created separately to the `Router`, such as having the remote address.

Create and store the `Router` as previous.

When you create the `Server`, pass in `&app.shared().with_state(state)` as the `router` to `Server::new`, where `app` is the stored `Router`.

The `Router` returned by `Router::shared` is lightweight and cheaply created and copied, so calling it per connection is cheap.

The "state_local" example demonstrates this pattern.

## [0.16.0] - 2025-05-13

### Breaking

- Split [`picoserve::Timeouts::start_read_request`](https://docs.rs/picoserve/0.16.0/picoserve/struct.Timeouts.html) into `start_read_request`, the timeout for reading the start of the first request, and `persistent_start_read_request`, the timeout for reading the start of subsequent requests on the same socket.

### Added

- Added support for accessing `State` when writing responses to the connection:
  - [`ContentUsingState`](https://docs.rs/picoserve/0.16.0/picoserve/response/with_state/trait.ContentUsingState.html) is the counterpart to [`Content`](https://docs.rs/picoserve/0.16.0/picoserve/response/trait.Content.html).
  - [`IntoResponseWithState`](https://docs.rs/picoserve/0.16.0/picoserve/response/with_state/trait.IntoResponseWithState.html) is the counterpart to [`IntoResponse`](https://docs.rs/picoserve/0.16.0/picoserve/response/trait.IntoResponse.html).
  - Added "response_using_state" example to demonstrate usage of new structures.

## [0.15.1] - 2025-03-26

### Fixed

- Fixed error message for enums deriving [`ErrorWithStatusCode`](https://docs.rs/picoserve/0.15.1/picoserve/response/trait.ErrorWithStatusCode.html) where neither the enum itself or one of its variants has a `status_code` attribute.
- [`SocketTx::send_pong`](https://docs.rs/picoserve/0.15.1/picoserve/response/ws/struct.SocketTx.html#method.send_ping) and [`SocketTx::send_ping`](https://docs.rs/picoserve/0.15.1/picoserve/response/ws/struct.SocketTx.html#method.send_pong) now flush.

### Added

- Added support for empty enums deriving [`ErrorWithStatusCode`](https://docs.rs/picoserve/0.15.1/picoserve/response/trait.ErrorWithStatusCode.html).
- Added support for generic structures deriving [`ErrorWithStatusCode`](https://docs.rs/picoserve/0.15.1/picoserve/response/trait.ErrorWithStatusCode.html).

## [0.15.0] - 2025-02-23

### Fixed

- Fixed type error in signiature of [`Router::nest`](https://docs.rs/picoserve/0.15.0/picoserve/routing/struct.Router.html#method.nest).

### Added

- Added [`Router::either_left_route`](https://docs.rs/picoserve/0.15.0/picoserve/routing/struct.Router.html#method.either_left_route) and [`Router::either_right_route`](https://docs.rs/picoserve/0.15.0/picoserve/routing/struct.Router.html#method.either_right_route) which can be used to create config-time conditional routers.
- Added support for [`response::Response`](https://docs.rs/picoserve/0.15.0/picoserve/response/struct.Response.html)s with no Content.
  For example responses with a code of 1xx (Informational) or 204 (No Content).
  - `(StatusCode, ..., NoContent,)` tuples now implement [`IntoResponse`](https://docs.rs/picoserve/0.15.0/picoserve/response/trait.IntoResponse.html)
  - Added [`Response::empty`](https://docs.rs/picoserve/0.15.0/picoserve/response/struct.Response.html#method.empty) to create a Response with no body.

### Changed

- When `embassy` feature is enabled, `serve` and `serve_with_state` accept any `S: io::Socket` rather than specifically a `embassy_net::tcp::TcpSocket<'_>`.

## [0.14.0] - 2025-01-20

### Breaking

- Updated to embassy-net >=0.6 and embassy-time >=0.4, which has a breaking change.

### Added

- Added derivable trait [`ErrorWithStatusCode`](https://docs.rs/picoserve/0.14.0/picoserve/response/trait.ErrorWithStatusCode.html) to facilitate creating error responses. Deriving `ErrorWithStatusCode` also derives [`IntoResponse`](https://docs.rs/picoserve/0.14.0/picoserve/response/trait.IntoResponse.html)

## [0.13.3] - 2024-12-26

### Fixed

- Require safety documentation for unsafe blocks.
- Fixed [FromRequest](https://docs.rs/picoserve/0.13.3/picoserve/extract/trait.FromRequest.html) for `alloc::vec::Vec`, and by extension, `alloc::string::String`.

### Changed

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

### Changed

- Request Bodies can not be either read into the internal buffer (as previously), or converted into a [`RequestBodyReader`](https://docs.rs/picoserve/0.9.0/picoserve/response/struct.RequestBodyReader.html), which implements Read.

### Added

- Added several unit tests around routing and reading requests.

## [0.8.1] - 2024-02-05

### Changed

- Fixed newline in WebSocketKeyHeaderMissing message.

## [0.8.0] - 2024-02-05

### Breaking

- [`serve`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve.html) and [`serve_with_state`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve_with_state.html) now take a socket rather than a reader and writer.

### Changed

- The socket is now shut down after it has finished handling requests

### Added

- Added support for [`embassy`](https://github.com/embassy-rs/embassy) with the `embassy` feature.
  - No need to declare and pass in a timer, used Embassy timers
  - Pass a [`TcpSocket`](https://docs.rs/embassy-net/0.4.0/embassy_net/tcp/struct.TcpSocket.html) to [`serve`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve.html) and [`serve_with_state`](https://docs.rs/picoserve/0.8.0/picoserve/fn.serve_with_state.html)
  - Added more examples which use embassy

## [0.7.2] - 2024-02-05

### Changed

- Using const_sha from crates.io (rather than copied into this repository) as it now has no_std support

## [0.7.1] - 2024-01-24

### Changed

- [Config::new](https://docs.rs/picoserve/0.7.1/picoserve/struct.Config.html#method.new) is now const

## [0.7.0] - 2024-01-20

### Fixed

- The "Connection" header is no longer sent in duplicate if the handler has already sent it

### Changed

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

### Changed

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

### Changed

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