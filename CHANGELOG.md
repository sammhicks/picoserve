# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0]

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