//! Route requests to the appropriate handler.
//!
//! At its core are "handler" functions, which are async functions with zero or more ["extractors"](crate::extract)

use core::{fmt, future::IntoFuture, marker::PhantomData, str::FromStr};

use crate::{
    extract::FromRequest,
    request::{Path, Request},
    response::{self, status, IntoResponse, ResponseWriter},
    ResponseSent,
};

mod layer;

use embedded_io_async::{ErrorType, Read, Write};
pub use layer::{Layer, Next};

#[doc(hidden)]
pub struct NoPathParameters;

#[doc(hidden)]
pub struct OnePathParameter<P>(pub P);

#[doc(hidden)]
pub struct ManyPathParameters<P>(pub P);

trait HandlerFunction<State, PathParameters, FunctionParameters, FunctionReturn> {
    async fn call_handler_func<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, WW::Error>;
}

struct BodyReader<R> {
    reader: R,
    content_length_left: usize,
}

impl<R: ErrorType> ErrorType for BodyReader<R> {
    type Error = R::Error;
}

impl<R: Read> Read for BodyReader<R> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, R::Error> {
        let len = buf.len().min(self.content_length_left);
        let buf = &mut buf[..len];

        if len == 0 {
            return Ok(0);
        }

        self.reader
            .read(buf)
            .await
            .inspect(|n| self.content_length_left -= n)
    }
}

impl<R: Read> BodyReader<R> {
    async fn finalize(mut self) -> Result<response::Connection<R>, R::Error> {
        let mut buf = [0; 64];
        while self.read(&mut buf).await? != 0 {}
        Ok(response::Connection(self.reader))
    }
}

macro_rules! declare_handler_func {
    ($($($name:ident)*;)*) => {
        $(
            impl<State, FunctionReturn: IntoFuture, $($name: FromRequest<State>,)* H: Fn($($name),*) -> FunctionReturn>
                HandlerFunction<State, NoPathParameters, ($($name,)*), FunctionReturn> for H
            where
                FunctionReturn::Output: IntoResponse,
            {
                #[allow(unused_variables)]
                async fn call_handler_func<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
                    &self,
                    state: &State,
                    path_parameters: NoPathParameters,
                    request: Request<'_>,
                    body_reader: R,
                    writer: WW,
                    response_writer: W,
                ) -> Result<ResponseSent, WW::Error> {
                    let mut body_reader = BodyReader {
                        reader: body_reader,
                        content_length_left: request.content_length(),
                    };

                    let res = (self)($(match <$name>::from_request(state, &request, &mut body_reader).await {
                        Ok(value) => value,
                        Err(err) => return err.write_to(writer, body_reader.finalize().await?, response_writer).await,
                    },)*)
                    .await;

                    res.write_to(writer, body_reader.finalize().await?, response_writer).await
                }
            }

            impl<State, PathParameter, FunctionReturn: IntoFuture, $($name: FromRequest<State>,)* H: Fn(PathParameter, $($name),*) -> FunctionReturn>
                HandlerFunction<State, OnePathParameter<PathParameter>, ($($name,)*), FunctionReturn> for H
            where
                FunctionReturn::Output: IntoResponse,
            {
                #[allow(unused_variables)]
                async fn call_handler_func<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
                    &self,
                    state: &State,
                    OnePathParameter(path_parameter): OnePathParameter<PathParameter>,
                    request: Request<'_>,
                    body_reader: R,
                    writer: WW,
                    response_writer: W,
                ) -> Result<ResponseSent, WW::Error> {
                    let mut body_reader = BodyReader {
                        reader: body_reader,
                        content_length_left: request.content_length(),
                    };
                    let res = (self)(
                        path_parameter,
                        $(match <$name>::from_request(state, &request, &mut body_reader).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(writer, body_reader.finalize().await?, response_writer).await,
                        },)*
                    )
                    .await;

                    res.write_to(writer, body_reader.finalize().await?, response_writer).await
                }
            }

            impl<State, PathParameters, FunctionReturn: IntoFuture, $($name: FromRequest<State>,)* H: Fn(PathParameters, $($name),*) -> FunctionReturn>
                HandlerFunction<State, ManyPathParameters<PathParameters>, ($($name,)*), FunctionReturn> for H
            where
                FunctionReturn::Output: IntoResponse,
            {
                #[allow(unused_variables)]
                async fn call_handler_func<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
                    &self,
                    state: &State,
                    ManyPathParameters(path_parameters): ManyPathParameters<PathParameters>,
                    request: Request<'_>,
                    body_reader: R,
                    writer: WW,
                    response_writer: W,
                ) -> Result<ResponseSent, WW::Error> {
                    let mut body_reader = BodyReader {
                        reader: body_reader,
                        content_length_left: request.content_length(),
                    };
                    let res = (self)(
                        path_parameters,
                        $(match <$name>::from_request(state, &request, &mut body_reader).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(writer, body_reader.finalize().await?, response_writer).await,
                        },)*
                    )
                    .await;

                    res.write_to(writer, body_reader.finalize().await?, response_writer).await
                }
            }
        )*
    };
}

declare_handler_func!(
    ;
    E1;
    E1 E2;
    E1 E2 E3;
    E1 E2 E3 E4;
    E1 E2 E3 E4 E5;
    E1 E2 E3 E4 E5 E6;
    E1 E2 E3 E4 E5 E6 E7;
    E1 E2 E3 E4 E5 E6 E7 E8;
    E1 E2 E3 E4 E5 E6 E7 E8 E9;
    E1 E2 E3 E4 E5 E6 E7 E8 E9 E10;
    E1 E2 E3 E4 E5 E6 E7 E8 E9 E10 E11;
    E1 E2 E3 E4 E5 E6 E7 E8 E9 E10 E11 E12;
    E1 E2 E3 E4 E5 E6 E7 E8 E9 E10 E11 E12 E13;
    E1 E2 E3 E4 E5 E6 E7 E8 E9 E10 E11 E12 E13 E14;
    E1 E2 E3 E4 E5 E6 E7 E8 E9 E10 E11 E12 E13 E14 E15;
    E1 E2 E3 E4 E5 E6 E7 E8 E9 E10 E11 E12 E13 E14 E15 E16;
);

/// Handles [Request]s, and writes the response to the provided [ResponseWriter].
pub trait RequestHandler<State, PathParameters> {
    async fn call_request_handler<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, WW::Error>;
}

#[doc(hidden)]
pub struct RequestHandlerFunctionCaller<FunctionParameters, FunctionReturn, Handler> {
    _params: PhantomData<fn(FunctionParameters) -> FunctionReturn>,
    handler: Handler,
}

impl<FunctionParameters, FunctionReturn, Handler>
    RequestHandlerFunctionCaller<FunctionParameters, FunctionReturn, Handler>
{
    fn new(handler: Handler) -> Self {
        Self {
            _params: PhantomData,
            handler,
        }
    }
}

impl<State, PathParameters, FunctionParameters, FunctionReturn, H>
    RequestHandler<State, PathParameters>
    for RequestHandlerFunctionCaller<FunctionParameters, FunctionReturn, H>
where
    H: HandlerFunction<State, PathParameters, FunctionParameters, FunctionReturn>,
{
    async fn call_request_handler<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, WW::Error> {
        self.handler
            .call_handler_func(
                state,
                path_parameters,
                request,
                body_reader,
                writer,
                response_writer,
            )
            .await
    }
}

/// [RequestHandler] for unsupported methods.
pub struct MethodNotAllowed;

impl<State, PathParameters> RequestHandler<State, PathParameters> for MethodNotAllowed {
    async fn call_request_handler<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        _state: &State,
        _path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, WW::Error> {
        (
            status::METHOD_NOT_ALLOWED,
            format_args!(
                "Method {} not allowed for {}\r\n",
                request.method(),
                request.path()
            ),
        )
            .write_to(writer, response::Connection(body_reader), response_writer)
            .await
    }
}

mod head_method_util {
    use embedded_io_async::{Read, Write};

    use crate::response::{Body, Connection, HeadersIter, Response, ResponseWriter};

    struct EmptyBody;

    impl Body for EmptyBody {
        async fn write_response_body<R: embedded_io_async::Read, W: Write<Error = R::Error>>(
            self,
            _connection: Connection<R>,
            _writer: W,
        ) -> Result<(), W::Error> {
            Ok(())
        }
    }

    struct IgnoreBody<W>(pub W);

    impl<W: ResponseWriter> ResponseWriter for IgnoreBody<W> {
        async fn write_response<H: HeadersIter, B: Body, WW: Write, R: Read<Error = WW::Error>>(
            self,
            writer: WW,
            connection: Connection<R>,
            Response {
                status_code,
                headers,
                body: _,
            }: Response<H, B>,
        ) -> Result<crate::ResponseSent, WW::Error> {
            self.0
                .write_response(
                    writer,
                    connection,
                    Response {
                        status_code,
                        headers,
                        body: EmptyBody,
                    },
                )
                .await
        }
    }

    pub fn ignore_body<W: ResponseWriter>(response_writer: W) -> impl ResponseWriter {
        IgnoreBody(response_writer)
    }
}

/// Routes a request based on its method.
pub trait MethodHandler<State, PathParameters> {
    async fn call_method_handler<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error>;
}

/// A [MethodHandler] which routes requests to the appropriate [RequestHandler] based on the method.
///
/// Automatically handled the `HEAD` method by calling the `GET` handler and returning an empty body.
pub struct MethodRouter<GET, POST> {
    get: GET,
    post: POST,
}

/// Route GET requests to the given handler.
pub fn get<State, PathParameters, FunctionParameters, FunctionReturn, Handler>(
    handler: Handler,
) -> MethodRouter<impl RequestHandler<State, PathParameters>, MethodNotAllowed>
where
    RequestHandlerFunctionCaller<FunctionParameters, FunctionReturn, Handler>:
        RequestHandler<State, PathParameters>,
{
    get_service(RequestHandlerFunctionCaller::new(handler))
}

/// Route GET requests to the given service.
pub fn get_service<State, PathParameters>(
    service: impl RequestHandler<State, PathParameters>,
) -> MethodRouter<impl RequestHandler<State, PathParameters>, MethodNotAllowed> {
    MethodRouter {
        get: service,
        post: MethodNotAllowed,
    }
}

/// Route POST requests to the given handler.
pub fn post<State, PathParameters, FunctionParameters, FunctionReturn, Handler>(
    handler: Handler,
) -> MethodRouter<MethodNotAllowed, impl RequestHandler<State, PathParameters>>
where
    RequestHandlerFunctionCaller<FunctionParameters, FunctionReturn, Handler>:
        RequestHandler<State, PathParameters>,
{
    post_service(RequestHandlerFunctionCaller::new(handler))
}

/// Route POST requests to the given service.
pub fn post_service<State, PathParameters>(
    service: impl RequestHandler<State, PathParameters>,
) -> MethodRouter<MethodNotAllowed, impl RequestHandler<State, PathParameters>> {
    MethodRouter {
        get: MethodNotAllowed,
        post: service,
    }
}

impl<POST> MethodRouter<MethodNotAllowed, POST> {
    pub fn get<State, PathParameters, FunctionParameters, FunctionReturn, Handler>(
        self,
        handler: Handler,
    ) -> MethodRouter<impl RequestHandler<State, PathParameters>, POST>
    where
        RequestHandlerFunctionCaller<FunctionParameters, FunctionReturn, Handler>:
            RequestHandler<State, PathParameters>,
    {
        self.get_service(RequestHandlerFunctionCaller::new(handler))
    }

    pub fn get_service<State, PathParameters>(
        self,
        service: impl RequestHandler<State, PathParameters>,
    ) -> MethodRouter<impl RequestHandler<State, PathParameters>, POST> {
        let MethodRouter {
            get: MethodNotAllowed,
            post,
        } = self;

        MethodRouter { get: service, post }
    }
}

impl<GET> MethodRouter<GET, MethodNotAllowed> {
    pub fn post<State, PathParameters, FunctionParameters, FunctionReturn, Handler>(
        self,
        handler: Handler,
    ) -> MethodRouter<GET, impl RequestHandler<State, PathParameters>>
    where
        RequestHandlerFunctionCaller<FunctionParameters, FunctionReturn, Handler>:
            RequestHandler<State, PathParameters>,
    {
        self.post_service(RequestHandlerFunctionCaller::new(handler))
    }

    pub fn post_service<State, PathParameters>(
        self,
        service: impl RequestHandler<State, PathParameters>,
    ) -> MethodRouter<GET, impl RequestHandler<State, PathParameters>> {
        let MethodRouter {
            get,
            post: MethodNotAllowed,
        } = self;

        MethodRouter { get, post: service }
    }
}

impl<GET, POST> MethodRouter<GET, POST> {
    pub fn layer<State, PathParameters, L: Layer<State, PathParameters>>(
        self,
        layer: L,
    ) -> impl MethodHandler<State, PathParameters>
    where
        GET: RequestHandler<L::NextState, L::NextPathParameters>,
        POST: RequestHandler<L::NextState, L::NextPathParameters>,
    {
        layer::MethodRouterLayer { layer, inner: self }
    }
}

impl<
        State,
        PathParameters,
        GET: RequestHandler<State, PathParameters>,
        POST: RequestHandler<State, PathParameters>,
    > MethodHandler<State, PathParameters> for MethodRouter<GET, POST>
{
    async fn call_method_handler<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        match request.method() {
            "GET" => {
                self.get
                    .call_request_handler(
                        state,
                        path_parameters,
                        request,
                        body_reader,
                        writer,
                        response_writer,
                    )
                    .await
            }
            "HEAD" => {
                self.get
                    .call_request_handler(
                        state,
                        path_parameters,
                        request,
                        body_reader,
                        writer,
                        head_method_util::ignore_body(response_writer),
                    )
                    .await
            }
            "POST" => {
                self.post
                    .call_request_handler(
                        state,
                        path_parameters,
                        request,
                        body_reader,
                        writer,
                        response_writer,
                    )
                    .await
            }
            _ => {
                MethodNotAllowed
                    .call_request_handler(
                        state,
                        path_parameters,
                        request,
                        body_reader,
                        writer,
                        response_writer,
                    )
                    .await
            }
        }
    }
}

/// Routes a request based on its path.
pub trait PathRouter<State = (), CurrentPathParameters = NoPathParameters> {
    async fn call_path_router<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error>;
}

/// [RequestHandler] for unhandled paths.
pub struct NotFound;

impl<State, CurrentPathParameters> PathRouter<State, CurrentPathParameters> for NotFound {
    async fn call_path_router<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        _state: &State,
        _current_path_parameters: CurrentPathParameters,
        _path: Path<'_>,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        (
            status::NOT_FOUND,
            format_args!("{} not found\r\n", request.path()),
        )
            .write_to(writer, response::Connection(body_reader), response_writer)
            .await
    }
}

#[doc(hidden)]
pub trait PathDescriptionBase: Copy + fmt::Debug {}

impl<T: Copy + fmt::Debug> PathDescriptionBase for T {}

/// A description of a path.
///
/// Typically one of:
/// + A string literal which the path is matched against, such as
///     + `/`
///     + `/foo`
///     + `/foo/bar`
/// + `parse_path_segment::<T>()`, which captures a single segment and tries to parse it using the `core::str::FromStr` implementation of `T`
/// + A tuple of types implementing PathDescription, thus allowing paths consisting of both static segments and captured segments, e.g.:
///     + `("/add", parse_path_segment::<i32>(), parse_path_segment::<i32>())`
///     + `("/user", parse_path_segment::<UserId>(), "/set_name", parse_path_segment::<UserName>())`
pub trait PathDescription<CurrentPathParameters>: PathDescriptionBase {
    /// The output of the parsed path description. Must implement [PushPathSegmentParameter] if not the final path description.
    type Output;

    fn parse_and_call<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        f: F,
    ) -> Result<T, CurrentPathParameters>;
}

impl<'a, CurrentPathParameters> PathDescription<CurrentPathParameters> for &'a str {
    type Output = CurrentPathParameters;

    fn parse_and_call<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        f: F,
    ) -> Result<T, CurrentPathParameters> {
        match path.strip_prefix(self) {
            Some(path) => f(current_path_parameters, path),
            None => Err(current_path_parameters),
        }
    }
}

/// The trait which powers concatinating several path parameters into a tuple of path parameters.
pub trait PushPathSegmentParameter<P>: Sized {
    type Output;

    fn push_path_segment_parameter_and_call<T, F: FnOnce(Self::Output) -> Result<T, Self::Output>>(
        self,
        segment: P,
        f: F,
    ) -> Result<T, Self>;
}

impl<P> PushPathSegmentParameter<P> for NoPathParameters {
    type Output = OnePathParameter<P>;

    fn push_path_segment_parameter_and_call<
        T,
        F: FnOnce(Self::Output) -> Result<T, Self::Output>,
    >(
        self,
        segment: P,
        f: F,
    ) -> Result<T, Self> {
        let NoPathParameters = self;

        f(OnePathParameter(segment)).map_err(|OnePathParameter(_)| NoPathParameters)
    }
}

impl<P, P1> PushPathSegmentParameter<P> for OnePathParameter<P1> {
    type Output = ManyPathParameters<(P1, P)>;

    fn push_path_segment_parameter_and_call<
        T,
        F: FnOnce(Self::Output) -> Result<T, Self::Output>,
    >(
        self,
        segment: P,
        f: F,
    ) -> Result<T, Self> {
        let OnePathParameter(p1) = self;

        f(ManyPathParameters((p1, segment)))
            .map_err(|ManyPathParameters((p1, _p))| OnePathParameter(p1))
    }
}

macro_rules! impl_tuple_push_path_segment_parameter {
    ($($($path_parameter:ident)*;)*) => {
        $(
            impl<$($path_parameter,)* P> PushPathSegmentParameter<P> for ManyPathParameters<($($path_parameter,)*)> {
                type Output = ManyPathParameters<($($path_parameter,)* P,)>;

                #[allow(non_snake_case)]
                fn push_path_segment_parameter_and_call<
                    T,
                    F: FnOnce(Self::Output) -> Result<T, Self::Output>,
                >(
                    self,
                    segment: P,
                    f: F,
                ) -> Result<T, Self> {
                    let ManyPathParameters(($($path_parameter,)*)) = self;

                    f(ManyPathParameters(($($path_parameter,)* segment,)))
                        .map_err(|ManyPathParameters(($($path_parameter,)* _p,))| ManyPathParameters(($($path_parameter,)*)))
                }
            }
        )*
    };
}

impl_tuple_push_path_segment_parameter!(
    ;
    P1;
    P1 P2;
    P1 P2 P3;
    P1 P2 P3 P4;
    P1 P2 P3 P4 P5;
    P1 P2 P3 P4 P5 P6;
    P1 P2 P3 P4 P5 P6 P7;
    P1 P2 P3 P4 P5 P6 P7 P8;
);

/// A [PathDescription] which parses a single segment using the implementation of `core::str::FromStr` of `T`.
pub struct ParsePathSegment<T>(PhantomData<T>);

impl<T> Clone for ParsePathSegment<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ParsePathSegment<T> {}

impl<T> fmt::Debug for ParsePathSegment<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ParsePath")
    }
}

/// Parse a single segment using the implementation of `core::str::FromStr` of `T`.
pub fn parse_path_segment<T: FromStr>() -> ParsePathSegment<T> {
    ParsePathSegment(PhantomData)
}

impl<CurrentPathParameters: PushPathSegmentParameter<P>, P: FromStr>
    PathDescription<CurrentPathParameters> for ParsePathSegment<P>
{
    type Output = CurrentPathParameters::Output;

    fn parse_and_call<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        f: F,
    ) -> Result<T, CurrentPathParameters> {
        let Some((segment, path)) = path.split_first_segment() else {
            return Err(current_path_parameters);
        };

        match segment
            .try_into_string::<128>()
            .ok()
            .and_then(|segment| segment.parse().ok())
        {
            Some(segment) => current_path_parameters
                .push_path_segment_parameter_and_call(segment, |path_parameters| {
                    f(path_parameters, path)
                }),
            None => Err(current_path_parameters),
        }
    }
}

impl<CurrentPathParameters> PathDescription<CurrentPathParameters> for () {
    type Output = CurrentPathParameters;

    fn parse_and_call<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        f: F,
    ) -> Result<T, CurrentPathParameters> {
        f(current_path_parameters, path)
    }
}

macro_rules! impl_tuple_path_description {
    ($($($name:ident)*;)*) => {
        $(
            impl<CurrentPathParameters, P: PathDescription<CurrentPathParameters> $(,$name: PathDescriptionBase)*>
                PathDescription<CurrentPathParameters> for (P, $($name,)*)
            where
                ($($name,)*): PathDescription<P::Output>,
            {
                type Output = <($($name,)*) as PathDescription<P::Output>>::Output;

                #[allow(non_snake_case)]
                fn parse_and_call<
                    'r,
                    T,
                    F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>,
                >(
                    &self,
                    current_path_parameters: CurrentPathParameters,
                    path: Path<'r>,
                    f: F,
                ) -> Result<T, CurrentPathParameters> {
                    let &(P, $($name,)*) = self;

                    P.parse_and_call(
                        current_path_parameters,
                        path,
                        |current_path_parameters, path| ($($name,)*).parse_and_call(current_path_parameters, path, f),
                    )
                }
            }
        )*
    };
}

impl_tuple_path_description!(
    ;
    P1;
    P1 P2;
    P1 P2 P3;
    P1 P2 P3 P4;
    P1 P2 P3 P4 P5;
    P1 P2 P3 P4 P5 P6;
    P1 P2 P3 P4 P5 P6 P7;
    P1 P2 P3 P4 P5 P6 P7 P8;
);

struct Route<PD, Handler, Fallback> {
    path_description: PD,
    handler: Handler,
    fallback: Fallback,
}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Handler: MethodHandler<State, PD::Output>,
        Fallback: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters> for Route<PD, Handler, Fallback>
{
    async fn call_path_router<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, WW::Error> {
        match self.path_description.parse_and_call(
            current_path_parameters,
            path,
            |path_parameters, path| {
                if path.0.is_empty() {
                    Ok(path_parameters)
                } else {
                    Err(path_parameters)
                }
            },
        ) {
            Ok(path_parameters) => {
                self.handler
                    .call_method_handler(
                        state,
                        path_parameters,
                        request,
                        body_reader,
                        writer,
                        response_writer,
                    )
                    .await
            }
            Err(current_path_parameters) => {
                self.fallback
                    .call_path_router(
                        state,
                        current_path_parameters,
                        path,
                        request,
                        body_reader,
                        writer,
                        response_writer,
                    )
                    .await
            }
        }
    }
}

struct NestedService<PD, Service, Fallback> {
    path_description: PD,
    service: Service,
    fallback: Fallback,
}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Service: PathRouter<State, PD::Output>,
        Fallback: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters> for NestedService<PD, Service, Fallback>
{
    async fn call_path_router<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_>,
        body_reader: R,
        writer: WW,
        response_writer: W,
    ) -> Result<ResponseSent, WW::Error> {
        match self.path_description.parse_and_call(
            current_path_parameters,
            path,
            |path_parameters, path| Ok((path_parameters, path)),
        ) {
            Ok((current_path_parameters, path)) => {
                self.service
                    .call_path_router(
                        state,
                        current_path_parameters,
                        path,
                        request,
                        body_reader,
                        writer,
                        response_writer,
                    )
                    .await
            }
            Err(current_path_parameters) => {
                self.fallback
                    .call_path_router(
                        state,
                        current_path_parameters,
                        path,
                        request,
                        body_reader,
                        writer,
                        response_writer,
                    )
                    .await
            }
        }
    }
}

/// A [PathRouter] which routes requests to a [MethodHandler].
pub struct Router<R, State = (), CurrentPathParameters = NoPathParameters> {
    pub(crate) router: R,
    _data: PhantomData<fn(CurrentPathParameters, State)>,
}

impl<State, CurrentPathParameters> Router<NotFound, State, CurrentPathParameters> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<State, CurrentPathParameters> Default for Router<NotFound, State, CurrentPathParameters> {
    fn default() -> Self {
        Self {
            router: NotFound,
            _data: PhantomData,
        }
    }
}

impl<State, CurrentPathParameters, R: PathRouter<State, CurrentPathParameters>>
    Router<R, State, CurrentPathParameters>
{
    pub fn route<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        handler: impl MethodHandler<State, PD::Output>,
    ) -> Router<impl PathRouter<State, CurrentPathParameters>, State, CurrentPathParameters> {
        let Router {
            router: fallback,
            _data,
        } = self;

        Router {
            router: Route {
                path_description,
                handler,
                fallback,
            },
            _data,
        }
    }

    pub fn nest<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        service: impl PathRouter<State, PD::Output>,
    ) -> Router<impl PathRouter<State, CurrentPathParameters>, State, CurrentPathParameters> {
        let Router {
            router: fallback,
            _data,
        } = self;

        Router {
            router: NestedService {
                path_description,
                service,
                fallback,
            },
            _data,
        }
    }

    pub fn layer<
        OuterState,
        OuterPathParameters,
        L: Layer<
            OuterState,
            OuterPathParameters,
            NextState = State,
            NextPathParameters = CurrentPathParameters,
        >,
    >(
        self,
        layer: L,
    ) -> Router<impl PathRouter<OuterState, OuterPathParameters>, OuterState, OuterPathParameters>
    {
        let Self {
            router: inner,
            _data,
        } = self;

        Router {
            router: layer::PathRouterLayer { layer, inner },
            _data: PhantomData,
        }
    }
}
