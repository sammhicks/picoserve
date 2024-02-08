# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
- Frame, Control, Data, and Message in [`response::ws`](https://docs.rs/picoserve/latest/picoserve/response/ws/index.html) now implement Debug

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