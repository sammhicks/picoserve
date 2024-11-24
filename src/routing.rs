//! Route requests to the appropriate handler.
//!
//! At its core are "handler" functions, which are async functions with zero or more ["extractors"](crate::extract) and which return ["responses"](crate::response::IntoResponse).
//! There are also "request handler services", which are types that implement ["RequestHandlerService"], such as:
//!     + [File](crate::response::fs::File)
//!     + [Directory](crate::response::fs::File)

use core::{fmt, future::IntoFuture, marker::PhantomData, str::FromStr};

use crate::{
    extract::{FromRequest, FromRequestParts},
    io::Read,
    request::{Path, Request},
    response::{IntoResponse, ResponseWriter, StatusCode},
    ResponseSent,
};

mod layer;

pub use layer::{Layer, Next};

mod sealed {
    pub trait Sealed {}
}

use sealed::Sealed;

#[doc(hidden)]
pub trait IntoPathParameterList: Sealed {
    type ParameterList;

    fn into_path_parameter_list(self) -> Self::ParameterList;
}

#[doc(hidden)]
pub struct NoPathParameters;

impl Sealed for NoPathParameters {}

impl IntoPathParameterList for NoPathParameters {
    type ParameterList = ();

    fn into_path_parameter_list(self) -> Self::ParameterList {}
}

#[doc(hidden)]
pub struct OnePathParameter<P>(pub P);

impl<P> Sealed for OnePathParameter<P> {}

impl<P> IntoPathParameterList for OnePathParameter<P> {
    type ParameterList = (P,);

    fn into_path_parameter_list(self) -> Self::ParameterList {
        (self.0,)
    }
}

#[doc(hidden)]
pub struct ManyPathParameters<P>(pub P);

impl<P> Sealed for ManyPathParameters<P> {}

impl<P> IntoPathParameterList for ManyPathParameters<P> {
    type ParameterList = P;

    fn into_path_parameter_list(self) -> Self::ParameterList {
        self.0
    }
}

/// Functions which can be used as a [RequestHandler].
pub trait RequestHandlerFunction<State, PathParameters, T> {
    /// Call the handler function and write the response to the [ResponseWriter].
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

impl<State, FunctionReturn: IntoFuture, H: Fn() -> FunctionReturn>
    RequestHandlerFunction<State, NoPathParameters, (FunctionReturn,)> for H
where
    FunctionReturn::Output: IntoResponse,
{
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &State,
        NoPathParameters: NoPathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (self)()
            .await
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

impl<State, PathParameter, FunctionReturn: IntoFuture, H: Fn(PathParameter) -> FunctionReturn>
    RequestHandlerFunction<State, OnePathParameter<PathParameter>, (FunctionReturn,)> for H
where
    FunctionReturn::Output: IntoResponse,
{
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &State,
        OnePathParameter(path_parameter): OnePathParameter<PathParameter>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (self)(path_parameter)
            .await
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

impl<
        State,
        PathParameters,
        FunctionReturn: IntoFuture,
        H: Fn(PathParameters) -> FunctionReturn,
    > RequestHandlerFunction<State, ManyPathParameters<PathParameters>, (FunctionReturn,)> for H
where
    FunctionReturn::Output: IntoResponse,
{
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &State,
        ManyPathParameters(path_parameters): ManyPathParameters<PathParameters>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (self)(path_parameters)
            .await
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

macro_rules! declare_handler_func {
    ($($($name:ident)*;)*) => {
        $(
            impl<State, FunctionReturn: IntoFuture, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: Fn($($name,)* E,) -> FunctionReturn>
                RequestHandlerFunction<State, NoPathParameters, (M, $($name,)* E, FunctionReturn,)> for H
            where
                FunctionReturn::Output: IntoResponse,
            {
                async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
                    &self,
                    state: &State,
                    NoPathParameters: NoPathParameters,
                    mut request: Request<'_, R>,
                    response_writer: W,
                ) -> Result<ResponseSent, W::Error> {
                    (self)(
                        $(match <$name>::from_request_parts(state, &request.parts).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(request.body_connection.finalize().await?, response_writer).await,
                        },)*
                        match E::from_request(state, request.parts, request.body_connection.body()).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(request.body_connection.finalize().await?, response_writer).await,
                        }
                    )
                    .await
                    .write_to(request.body_connection.finalize().await?, response_writer)
                    .await
                }
            }

            impl<State, PathParameter, FunctionReturn: IntoFuture, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: Fn(PathParameter, $($name,)* E,) -> FunctionReturn>
                RequestHandlerFunction<State, OnePathParameter<PathParameter>, (M, $($name,)* E, FunctionReturn,)> for H
            where
                FunctionReturn::Output: IntoResponse,
            {
                #[allow(unused_variables)]
                async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
                    &self,
                    state: &State,
                    OnePathParameter(path_parameter): OnePathParameter<PathParameter>,
                    mut request: Request<'_, R>,
                    response_writer: W,
                ) -> Result<ResponseSent, W::Error> {
                    (self)(
                        path_parameter,
                        $(match <$name>::from_request_parts(state, &request.parts).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(request.body_connection.finalize().await?, response_writer).await,
                        },)*
                        match E::from_request(state, request.parts, request.body_connection.body()).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(request.body_connection.finalize().await?, response_writer).await,
                        }
                    )
                    .await
                    .write_to(request.body_connection.finalize().await?, response_writer)
                    .await
                }
            }

            impl<State, PathParameters, FunctionReturn: IntoFuture, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: Fn(PathParameters, $($name,)* E,) -> FunctionReturn>
                RequestHandlerFunction<State, ManyPathParameters<PathParameters>, (M, $($name,)* E, FunctionReturn)> for H
            where
                FunctionReturn::Output: IntoResponse,
            {
                #[allow(unused_variables)]
                async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
                    &self,
                    state: &State,
                    ManyPathParameters(path_parameters): ManyPathParameters<PathParameters>,
                    mut request: Request<'_, R>,
                    response_writer: W,
                ) -> Result<ResponseSent, W::Error> {
                    (self)(
                        path_parameters,
                        $(match <$name>::from_request_parts(state, &request.parts).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(request.body_connection.finalize().await?, response_writer).await,
                        },)*
                        match E::from_request(state, request.parts, request.body_connection.body()).await {
                            Ok(value) => value,
                            Err(err) => return err.write_to(request.body_connection.finalize().await?, response_writer).await,
                        }
                    )
                    .await
                    .write_to(request.body_connection.finalize().await?, response_writer)
                    .await
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

/// Handles [Request]s and writes the response to the provided [ResponseWriter].
pub trait RequestHandler<State, PathParameters>: Sealed {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_request_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

struct HandlerFunctionRequestHandler<T, Handler> {
    phantom_data: PhantomData<fn(&T)>,
    handler: Handler,
}

impl<T, Handler> Sealed for HandlerFunctionRequestHandler<T, Handler> {}

impl<T, Handler> HandlerFunctionRequestHandler<T, Handler> {
    fn new(handler: Handler) -> Self {
        Self {
            phantom_data: PhantomData,
            handler,
        }
    }
}

impl<State, PathParameters, T, H: RequestHandlerFunction<State, PathParameters, T>>
    RequestHandler<State, PathParameters> for HandlerFunctionRequestHandler<T, H>
{
    async fn call_request_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.handler
            .call_handler_func(state, path_parameters, request, response_writer)
            .await
    }
}

/// A service which handles [Request]s and writes the response to the provided [ResponseWriter].
pub trait RequestHandlerService<State, PathParameters = ()> {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

struct RequestHandlerServiceRequestHandler<Service> {
    service: Service,
}

impl<Service> Sealed for RequestHandlerServiceRequestHandler<Service> {}

impl<
        State,
        PathParameters: IntoPathParameterList,
        Service: RequestHandlerService<State, PathParameters::ParameterList>,
    > RequestHandler<State, PathParameters> for RequestHandlerServiceRequestHandler<Service>
{
    async fn call_request_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.service
            .call_request_handler_service(
                state,
                path_parameters.into_path_parameter_list(),
                request,
                response_writer,
            )
            .await
    }
}

/// [RequestHandler] for unsupported methods.
pub struct MethodNotAllowed;

impl Sealed for MethodNotAllowed {}

impl<State, PathParameters> RequestHandler<State, PathParameters> for MethodNotAllowed {
    async fn call_request_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &State,
        _path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            StatusCode::METHOD_NOT_ALLOWED,
            format_args!(
                "Method {} not allowed for {}\r\n",
                request.parts.method(),
                request.parts.path()
            ),
        )
            .write_to(request.body_connection.finalize().await?, response_writer)
            .await
    }
}

mod head_method_util {
    use embedded_io_async::Write;

    use crate::response::{Body, Connection, HeadersIter, Response, ResponseWriter};

    struct EmptyBody;

    impl Body for EmptyBody {
        async fn write_response_body<R: embedded_io_async::Read, W: Write<Error = R::Error>>(
            self,
            _connection: Connection<'_, R>,
            _writer: W,
        ) -> Result<(), W::Error> {
            Ok(())
        }
    }

    struct IgnoreBody<W>(pub W);

    impl<W: ResponseWriter> ResponseWriter for IgnoreBody<W> {
        type Error = W::Error;

        async fn write_response<
            R: embedded_io_async::Read<Error = Self::Error>,
            H: HeadersIter,
            B: Body,
        >(
            self,
            connection: Connection<'_, R>,
            Response {
                status_code,
                headers,
                body: _,
            }: Response<H, B>,
        ) -> Result<crate::ResponseSent, Self::Error> {
            self.0
                .write_response(
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

    pub fn ignore_body<W: ResponseWriter>(
        response_writer: W,
    ) -> impl ResponseWriter<Error = W::Error> {
        IgnoreBody(response_writer)
    }
}

/// Routes a request based on its method.
pub trait MethodHandler<State, PathParameters>: Sealed {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_method_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

/// A [MethodHandler] which routes requests to the appropriate [RequestHandler] based on the method.
///
/// Automatically handled the `HEAD` method by calling the `GET` handler and returning an empty body.
pub struct MethodRouter<GET, POST> {
    get: GET,
    post: POST,
}

impl<GET, POST> Sealed for MethodRouter<GET, POST> {}

/// Route `GET` requests to the given [handler](RequestHandlerFunction).
pub fn get<State, PathParameters, T, Handler: RequestHandlerFunction<State, PathParameters, T>>(
    handler: Handler,
) -> MethodRouter<impl RequestHandler<State, PathParameters>, MethodNotAllowed> {
    MethodRouter {
        get: HandlerFunctionRequestHandler::new(handler),
        post: MethodNotAllowed,
    }
}

/// Route `GET` requests to the given [service](RequestHandlerService).
pub fn get_service<State, PathParameters: IntoPathParameterList>(
    service: impl RequestHandlerService<State, PathParameters::ParameterList>,
) -> MethodRouter<impl RequestHandler<State, PathParameters>, MethodNotAllowed> {
    MethodRouter {
        get: RequestHandlerServiceRequestHandler { service },
        post: MethodNotAllowed,
    }
}

/// Route `POST` requests to the given [handler](RequestHandlerFunction).
pub fn post<State, PathParameters, T, Handler: RequestHandlerFunction<State, PathParameters, T>>(
    handler: Handler,
) -> MethodRouter<MethodNotAllowed, impl RequestHandler<State, PathParameters>> {
    MethodRouter {
        get: MethodNotAllowed,
        post: HandlerFunctionRequestHandler::new(handler),
    }
}

/// Route `POST` requests to the given [service](RequestHandlerService).
pub fn post_service<State, PathParameters: IntoPathParameterList>(
    service: impl RequestHandlerService<State, PathParameters::ParameterList>,
) -> MethodRouter<MethodNotAllowed, impl RequestHandler<State, PathParameters>> {
    MethodRouter {
        get: MethodNotAllowed,
        post: RequestHandlerServiceRequestHandler { service },
    }
}

impl<POST> MethodRouter<MethodNotAllowed, POST> {
    /// Chain an additional [handler](RequestHandlerFunction) that will only accept `GET` requests.
    pub fn get<
        State,
        PathParameters,
        T,
        Handler: RequestHandlerFunction<State, PathParameters, T>,
    >(
        self,
        handler: Handler,
    ) -> MethodRouter<impl RequestHandler<State, PathParameters>, POST> {
        let MethodRouter {
            get: MethodNotAllowed,
            post,
        } = self;

        MethodRouter {
            get: HandlerFunctionRequestHandler::new(handler),
            post,
        }
    }

    /// Chain an additional [service](RequestHandlerService) that will only accept `GET` requests.
    pub fn get_service<State, PathParameters: IntoPathParameterList>(
        self,
        service: impl RequestHandlerService<State, PathParameters::ParameterList>,
    ) -> MethodRouter<impl RequestHandler<State, PathParameters>, POST> {
        let MethodRouter {
            get: MethodNotAllowed,
            post,
        } = self;

        MethodRouter {
            get: RequestHandlerServiceRequestHandler { service },
            post,
        }
    }
}

impl<GET> MethodRouter<GET, MethodNotAllowed> {
    /// Chain an additional [handler](RequestHandlerFunction) that will only accept `POST` requests.
    pub fn post<
        State,
        PathParameters,
        T,
        Handler: RequestHandlerFunction<State, PathParameters, T>,
    >(
        self,
        handler: Handler,
    ) -> MethodRouter<GET, impl RequestHandler<State, PathParameters>> {
        let MethodRouter {
            get,
            post: MethodNotAllowed,
        } = self;

        MethodRouter {
            get,
            post: HandlerFunctionRequestHandler::new(handler),
        }
    }

    /// Chain an additional [service](RequestHandlerService) that will only accept `POST` requests.
    pub fn post_service<State, PathParameters: IntoPathParameterList>(
        self,
        service: impl RequestHandlerService<State, PathParameters::ParameterList>,
    ) -> MethodRouter<GET, impl RequestHandler<State, PathParameters>> {
        let MethodRouter {
            get,
            post: MethodNotAllowed,
        } = self;

        MethodRouter {
            get,
            post: RequestHandlerServiceRequestHandler { service },
        }
    }
}

impl<GET, POST> MethodRouter<GET, POST> {
    /// Add a [Layer] to all routes in the router
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
    async fn call_method_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match request.parts.method() {
            "GET" => {
                self.get
                    .call_request_handler(state, path_parameters, request, response_writer)
                    .await
            }
            "HEAD" => {
                self.get
                    .call_request_handler(
                        state,
                        path_parameters,
                        request,
                        head_method_util::ignore_body(response_writer),
                    )
                    .await
            }
            "POST" => {
                self.post
                    .call_request_handler(state, path_parameters, request, response_writer)
                    .await
            }
            _ => {
                MethodNotAllowed
                    .call_request_handler(state, path_parameters, request, response_writer)
                    .await
            }
        }
    }
}

/// Routes a request based on its path.
pub trait PathRouter<State = (), CurrentPathParameters = NoPathParameters>: Sealed {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

/// [RequestHandler] for unhandled paths.
pub struct NotFound;

impl Sealed for NotFound {}

impl<State, CurrentPathParameters> PathRouter<State, CurrentPathParameters> for NotFound {
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &State,
        _current_path_parameters: CurrentPathParameters,
        _path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (
            StatusCode::NOT_FOUND,
            format_args!("{} not found\r\n", request.parts.path()),
        )
            .write_to(request.body_connection.finalize().await?, response_writer)
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

    /// Parse the path.
    fn parse<'r>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
    ) -> Result<(Self::Output, Path<'r>), CurrentPathParameters> {
        self.parse_and_validate(current_path_parameters, path, |path_parameters, path| {
            Ok((path_parameters, path))
        })
    }

    /// Parse the path and then call the validation function.
    fn parse_and_validate<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        validate: F,
    ) -> Result<T, CurrentPathParameters>;
}

impl<'a, CurrentPathParameters> PathDescription<CurrentPathParameters> for &'a str {
    type Output = CurrentPathParameters;

    fn parse_and_validate<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
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
pub trait PushPathSegmentParameter<P>: Sealed + Sized {
    /// The concatenation of the current value and the new value
    type Output;

    /// Concatenate the given segment and validate the result
    fn push_path_segment_parameter_and_validate<
        T,
        F: FnOnce(Self::Output) -> Result<T, Self::Output>,
    >(
        self,
        segment: P,
        validate: F,
    ) -> Result<T, Self>;
}

impl<P> PushPathSegmentParameter<P> for NoPathParameters {
    type Output = OnePathParameter<P>;

    fn push_path_segment_parameter_and_validate<
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

    fn push_path_segment_parameter_and_validate<
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
                fn push_path_segment_parameter_and_validate<
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

    fn parse_and_validate<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
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
                .push_path_segment_parameter_and_validate(segment, |path_parameters| {
                    f(path_parameters, path)
                }),
            None => Err(current_path_parameters),
        }
    }
}

impl<CurrentPathParameters> PathDescription<CurrentPathParameters> for () {
    type Output = CurrentPathParameters;

    fn parse_and_validate<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
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
                fn parse_and_validate<
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

                    P.parse_and_validate(
                        current_path_parameters,
                        path,
                        |current_path_parameters, path| ($($name,)*).parse_and_validate(current_path_parameters, path, f),
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

impl<PD, Handler, Fallback> Sealed for Route<PD, Handler, Fallback> {}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Handler: MethodHandler<State, PD::Output>,
        Fallback: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters> for Route<PD, Handler, Fallback>
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self.path_description.parse_and_validate(
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
                    .call_method_handler(state, path_parameters, request, response_writer)
                    .await
            }
            Err(current_path_parameters) => {
                self.fallback
                    .call_path_router(
                        state,
                        current_path_parameters,
                        path,
                        request,
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

impl<PD, Service, Fallback> Sealed for NestedService<PD, Service, Fallback> {}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Service: PathRouter<State, PD::Output>,
        Fallback: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters> for NestedService<PD, Service, Fallback>
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self.path_description.parse(current_path_parameters, path) {
            Ok((current_path_parameters, path)) => {
                self.service
                    .call_path_router(
                        state,
                        current_path_parameters,
                        path,
                        request,
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
                        response_writer,
                    )
                    .await
            }
        }
    }
}

/// A service which handles both path routing and subsequent request handling.
pub trait PathRouterService<State, CurrentPathParameters = ()> {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_request_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

struct PathRouterServicePathRouter<PD, Service, Fallback> {
    path_description: PD,
    service: Service,
    fallback: Fallback,
}

impl<PD, Service, Fallback> Sealed for PathRouterServicePathRouter<PD, Service, Fallback> {}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Service: PathRouterService<State, <<PD as PathDescription<CurrentPathParameters>>::Output as IntoPathParameterList>::ParameterList>,
        Fallback: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters>
    for PathRouterServicePathRouter<PD, Service, Fallback>
where
    <PD as PathDescription<CurrentPathParameters>>::Output: IntoPathParameterList,
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self.path_description.parse(
            current_path_parameters,
            path,
        ) {
            Ok((path_parameters, path)) => {
                self.service
                    .call_request_handler_service(
                        state,
                        path_parameters.into_path_parameter_list(),
                        path,
                        request,
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
                        response_writer,
                    )
                    .await
            }
        }
    }
}

/// A [PathRouter] which forwards all requests to the provided [PathRouterService]
pub struct ServicePathRouter<Service> {
    service: Service,
}

impl<Service> Sealed for ServicePathRouter<Service> {}

impl<
        State,
        CurrentPathParameters: IntoPathParameterList,
        Service: PathRouterService<State, <CurrentPathParameters as IntoPathParameterList>::ParameterList>,
    > PathRouter<State, CurrentPathParameters> for ServicePathRouter<Service>
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.service
            .call_request_handler_service(
                state,
                current_path_parameters.into_path_parameter_list(),
                path,
                request,
                response_writer,
            )
            .await
    }
}

/// A [PathRouter] which routes requests to a [MethodHandler].
pub struct Router<RouterInner, State = (), CurrentPathParameters = NoPathParameters> {
    pub(crate) router: RouterInner,
    _data: PhantomData<fn(CurrentPathParameters, State)>,
}

impl<State, CurrentPathParameters> Router<NotFound, State, CurrentPathParameters> {
    /// Create a new `Router`, which returns `404 Not Found` to all requests.
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

impl<
        State,
        CurrentPathParameters: IntoPathParameterList,
        Service: PathRouterService<State, <CurrentPathParameters as IntoPathParameterList>::ParameterList>,
    > Router<ServicePathRouter<Service>, State, CurrentPathParameters>
{
    /// Create a [Router] which forwards all requests to the provided [PathRouterService].
    pub fn from_service(service: Service) -> Self {
        Self {
            router: ServicePathRouter { service },
            _data: PhantomData,
        }
    }
}

impl<State, CurrentPathParameters, RouterInner: PathRouter<State, CurrentPathParameters>>
    Router<RouterInner, State, CurrentPathParameters>
{
    /// Add another route to the router
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

    /// Nest a [Router] at some path
    pub fn nest<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        router: Router<impl PathRouter<State, PD::Output>, State>,
    ) -> Router<impl PathRouter<State, CurrentPathParameters>, State, CurrentPathParameters> {
        let Router {
            router: fallback,
            _data,
        } = self;

        Router {
            router: NestedService {
                path_description,
                service: router.router,
                fallback,
            },
            _data,
        }
    }

    /// Nest a [PathRouterService] at some path, like [nest](Self::nest) but accepts an arbitary service
    pub fn nest_service<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        service: impl PathRouterService<State, <PD::Output as IntoPathParameterList>::ParameterList>,
    ) -> Router<impl PathRouter<State, CurrentPathParameters>, State, CurrentPathParameters>
    where
        PD::Output: IntoPathParameterList,
    {
        let Router {
            router: fallback,
            _data,
        } = self;

        Router {
            router: PathRouterServicePathRouter {
                path_description,
                service,
                fallback,
            },
            _data,
        }
    }

    /// Apply a [Layer] to all routes in the router.
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

    pub async fn handle_request<R: Read<Error = W::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.router
            .call_path_router(
                state,
                current_path_parameters,
                request.parts.path(),
                request,
                response_writer,
            )
            .await
    }
}
