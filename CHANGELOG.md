# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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