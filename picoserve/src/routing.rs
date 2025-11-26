//! Route requests to the appropriate handler.
//!
//! At its core are "handler" functions, which are async functions with zero or more ["extractors"](crate::extract) and which return ["responses"](crate::response::IntoResponse).
//! There are also "request handler services", which are types that implement ["RequestHandlerService"], such as:
//!     + [File](crate::response::fs::File)
//!     + [Directory](crate::response::fs::File)

use core::{fmt, marker::PhantomData, str::FromStr};

use crate::{
    extract::{FromRequest, FromRequestParts},
    io::Read,
    request::{Path, Request},
    response::{with_state::IntoResponseWithState, IntoResponse, ResponseWriter, StatusCode},
    ResponseSent,
};

mod layer;

pub use layer::{Layer, Next};

mod sealed {
    /// Only `picoserve` may declare types which implement [`RequestHandlerFunction`](super::RequestHandlerFunction).
    pub trait RequestHandlerFunctionIsSealed<State, PathParameters, HandlerTypeSigniature> {}

    /// Only `picoserve` may create types which implement [`RequestHandler`](super::RequestHandler).
    pub trait RequestHandlerIsSealed {}

    /// Only `picoserve` may create types which implement [`MethodHandler`](super::MethodHandler).
    pub trait MethodHandlerIsSealed {}

    /// Only `picoserve` may create types which implement [`PathRouter`](super::PathRouter).
    pub trait PathRouterIsSealed {}

    /// Only `picoserve` may declare types which implement [`PushPathSegmentParameter`](super::PushPathSegmentParameter).
    pub trait PushPathSegmentParameterIsSealed {}
}

mod request_handler_function_components {
    pub struct OnePathParameter<P>(core::marker::PhantomData<(P,)>);
    pub struct ManyPathParameters<P>(core::marker::PhantomData<P>);
    pub struct ParametersFromRequestParts<E>(core::marker::PhantomData<fn() -> E>);
    pub struct ParameterFromRequest<M, E>(core::marker::PhantomData<fn(&M) -> E>);
}

use request_handler_function_components::{
    ManyPathParameters, OnePathParameter, ParameterFromRequest, ParametersFromRequestParts,
};

/// Functions which can be used as a [RequestHandler].
pub trait RequestHandlerFunction<State, PathParameters, HandlerTypeSigniature>:
    sealed::RequestHandlerFunctionIsSealed<State, PathParameters, HandlerTypeSigniature>
{
    /// Call the handler function and write the response to the [ResponseWriter].
    async fn call_request_handler_function<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

impl<State, FunctionReturn: IntoResponseWithState<State>, H: AsyncFn() -> FunctionReturn>
    sealed::RequestHandlerFunctionIsSealed<State, (), (FunctionReturn,)> for H
{
}

impl<State, FunctionReturn: IntoResponseWithState<State>, H: AsyncFn() -> FunctionReturn>
    RequestHandlerFunction<State, (), (FunctionReturn,)> for H
{
    async fn call_request_handler_function<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        (): (),
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (self)()
            .await
            .write_to_with_state(
                state,
                request.body_connection.finalize().await?,
                response_writer,
            )
            .await
    }
}

impl<
        State,
        PathParameter,
        FunctionReturn: IntoResponseWithState<State>,
        H: AsyncFn(PathParameter) -> FunctionReturn,
    >
    sealed::RequestHandlerFunctionIsSealed<
        State,
        (PathParameter,),
        (OnePathParameter<PathParameter>, FunctionReturn),
    > for H
{
}

impl<
        State,
        PathParameter,
        FunctionReturn: IntoResponseWithState<State>,
        H: AsyncFn(PathParameter) -> FunctionReturn,
    >
    RequestHandlerFunction<
        State,
        (PathParameter,),
        (OnePathParameter<PathParameter>, FunctionReturn),
    > for H
{
    async fn call_request_handler_function<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        (path_parameter,): (PathParameter,),
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (self)(path_parameter)
            .await
            .write_to_with_state(
                state,
                request.body_connection.finalize().await?,
                response_writer,
            )
            .await
    }
}

impl<
        State,
        PathParameters,
        FunctionReturn: IntoResponseWithState<State>,
        H: AsyncFn(PathParameters) -> FunctionReturn,
    >
    sealed::RequestHandlerFunctionIsSealed<
        State,
        PathParameters,
        (ManyPathParameters<PathParameters>, FunctionReturn),
    > for H
{
}

impl<
        State,
        PathParameters,
        FunctionReturn: IntoResponseWithState<State>,
        H: AsyncFn(PathParameters) -> FunctionReturn,
    >
    RequestHandlerFunction<
        State,
        PathParameters,
        (ManyPathParameters<PathParameters>, FunctionReturn),
    > for H
{
    async fn call_request_handler_function<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        (self)(path_parameters)
            .await
            .write_to_with_state(
                state,
                request.body_connection.finalize().await?,
                response_writer,
            )
            .await
    }
}

macro_rules! declare_handler_func {
    ($($($name:ident)*;)*) => {
        $(
            impl<State, FunctionReturn: IntoResponseWithState<State>, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: AsyncFn($($name,)* E) -> FunctionReturn>
                sealed::RequestHandlerFunctionIsSealed<State, (), (ParametersFromRequestParts<($($name,)*)>, ParameterFromRequest<M, E>, FunctionReturn)> for H
            {
            }

            impl<State, FunctionReturn: IntoResponseWithState<State>, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: AsyncFn($($name,)* E) -> FunctionReturn>
                RequestHandlerFunction<State, (), (ParametersFromRequestParts<($($name,)*)>, ParameterFromRequest<M, E>, FunctionReturn)> for H
            {
                async fn call_request_handler_function<R: Read, W: ResponseWriter<Error = R::Error>>(
                    &self,
                    state: &State,
                    (): (),
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
                    .write_to_with_state(state, request.body_connection.finalize().await?, response_writer)
                    .await
                }
            }

            impl<State, PathParameter, FunctionReturn: IntoResponseWithState<State>, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: AsyncFn(PathParameter, $($name,)* E,) -> FunctionReturn>
                sealed::RequestHandlerFunctionIsSealed<State, (PathParameter,), (OnePathParameter<PathParameter>, ParametersFromRequestParts<($($name,)*)>, ParameterFromRequest<M, E>,  FunctionReturn,)> for H
            {
            }

            impl<State, PathParameter, FunctionReturn: IntoResponseWithState<State>, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: AsyncFn(PathParameter, $($name,)* E,) -> FunctionReturn>
                RequestHandlerFunction<State, (PathParameter,), (OnePathParameter<PathParameter>, ParametersFromRequestParts<($($name,)*)>, ParameterFromRequest<M, E>,  FunctionReturn,)> for H
            {
                #[allow(unused_variables)]
                async fn call_request_handler_function<R: Read, W: ResponseWriter<Error = R::Error>>(
                    &self,
                    state: &State,
                    (path_parameter,): (PathParameter,),
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
                    .write_to_with_state(state, request.body_connection.finalize().await?, response_writer)
                    .await
                }
            }

            impl<State, PathParameters, FunctionReturn: IntoResponseWithState<State>, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: AsyncFn(PathParameters, $($name,)* E,) -> FunctionReturn>
                sealed::RequestHandlerFunctionIsSealed<State, PathParameters, (ManyPathParameters<PathParameters>, ParametersFromRequestParts<($($name,)*)>, ParameterFromRequest<M, E>,  FunctionReturn)> for H
            {}

            impl<State, PathParameters, FunctionReturn: IntoResponseWithState<State>, $($name: for<'a> FromRequestParts<'a, State>,)* M, E: for<'a> FromRequest<'a, State, M>, H: AsyncFn(PathParameters, $($name,)* E,) -> FunctionReturn>
                RequestHandlerFunction<State, PathParameters, (ManyPathParameters<PathParameters>, ParametersFromRequestParts<($($name,)*)>, ParameterFromRequest<M, E>,  FunctionReturn)> for H
            {
                #[allow(unused_variables)]
                async fn call_request_handler_function<R: Read, W: ResponseWriter<Error = R::Error>>(
                    &self,
                    state: &State,
                    path_parameters: PathParameters,
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
                    .write_to_with_state(state, request.body_connection.finalize().await?, response_writer)
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
pub trait RequestHandler<State, PathParameters>: sealed::RequestHandlerIsSealed {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_request_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

struct HandlerFunctionRequestHandler<Handler, HandlerTypeSigniature> {
    handler: Handler,
    _handler_type_signiature: PhantomData<fn(&HandlerTypeSigniature)>,
}

impl<Handler, HandlerTypeSigniature> sealed::RequestHandlerIsSealed
    for HandlerFunctionRequestHandler<Handler, HandlerTypeSigniature>
{
}

impl<Handler, HandlerTypeSigniature> HandlerFunctionRequestHandler<Handler, HandlerTypeSigniature> {
    fn new(handler: Handler) -> Self {
        Self {
            handler,
            _handler_type_signiature: PhantomData,
        }
    }
}

impl<
        State,
        PathParameters,
        HandlerTypeSigniature,
        Handler: RequestHandlerFunction<State, PathParameters, HandlerTypeSigniature>,
    > RequestHandler<State, PathParameters>
    for HandlerFunctionRequestHandler<Handler, HandlerTypeSigniature>
{
    async fn call_request_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.handler
            .call_request_handler_function(state, path_parameters, request, response_writer)
            .await
    }
}

/// A service which handles [Request]s and writes the response to the provided [ResponseWriter].
pub trait RequestHandlerService<State = (), PathParameters = ()> {
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

impl<Service> sealed::RequestHandlerIsSealed for RequestHandlerServiceRequestHandler<Service> {}

impl<State, PathParameters, Service: RequestHandlerService<State, PathParameters>>
    RequestHandler<State, PathParameters> for RequestHandlerServiceRequestHandler<Service>
{
    async fn call_request_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.service
            .call_request_handler_service(state, path_parameters, request, response_writer)
            .await
    }
}

/// [RequestHandler] for unsupported methods.
pub struct MethodNotAllowed;

impl sealed::RequestHandlerIsSealed for MethodNotAllowed {}

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
    use crate::{
        io::{Read, Write},
        response::{Body, Connection, HeadersIter, Response, ResponseWriter},
    };

    struct EmptyBody;

    impl Body for EmptyBody {
        async fn write_response_body<R: Read, W: Write<Error = R::Error>>(
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

        async fn write_response<R: Read<Error = Self::Error>, H: HeadersIter, B: Body>(
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
pub trait MethodHandler<State = (), PathParameters = ()>: sealed::MethodHandlerIsSealed {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_method_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

picoserve_derive::generate_method_router!(get, post, put, delete, options, trace, patch);

/// Routes a request based on its path.
pub trait PathRouter<State = (), CurrentPathParameters = ()>: sealed::PathRouterIsSealed {
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

impl sealed::PathRouterIsSealed for NotFound {}

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
pub trait PathDescription<CurrentPathParameters>: Copy + fmt::Debug {
    /// The current path parameters, and the new path parameter, if any.
    type NewPathParameters;

    /// Parse the section of the path described by `Self`` and then call the validation function with the new path parameters and the rest of the path.
    fn parse_and_validate<
        'r,
        T,
        F: FnOnce(Self::NewPathParameters, Path<'r>) -> Result<T, Self::NewPathParameters>,
    >(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        validate: F,
    ) -> Result<T, CurrentPathParameters>;

    /// Parse the entire path, verifying that Self describes the entire path.
    fn parse_entire_path(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
    ) -> Result<Self::NewPathParameters, CurrentPathParameters> {
        self.parse_and_validate(
            current_path_parameters,
            path,
            |new_path_parameters, path| {
                if path.0.is_empty() {
                    Ok(new_path_parameters)
                } else {
                    Err(new_path_parameters)
                }
            },
        )
    }

    /// Parse the prefix of the path described by `Self`, verifying that the rest of the path isn't empty.
    fn parse_path_prefix<'r>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
    ) -> Result<(Self::NewPathParameters, Path<'r>), CurrentPathParameters> {
        self.parse_and_validate(
            current_path_parameters,
            path,
            |new_path_parameters, path| {
                if path.0.is_empty() {
                    Err(new_path_parameters)
                } else {
                    Ok((new_path_parameters, path))
                }
            },
        )
    }
}

impl<CurrentPathParameters> PathDescription<CurrentPathParameters> for &str {
    type NewPathParameters = CurrentPathParameters;

    fn parse_and_validate<
        'r,
        T,
        F: FnOnce(Self::NewPathParameters, Path<'r>) -> Result<T, Self::NewPathParameters>,
    >(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        validate: F,
    ) -> Result<T, CurrentPathParameters> {
        match path.strip_prefix(self) {
            Some(path) => validate(current_path_parameters, path),
            None => Err(current_path_parameters),
        }
    }
}

/// The trait which powers concatinating several path parameters into a tuple of path parameters.
pub trait PushPathSegmentParameter<P>: sealed::PushPathSegmentParameterIsSealed + Sized {
    /// The concatenation of the current path parameters and the new path parameter.
    type NewPathParameters;

    /// Push a new segment parameter to the end of the list.
    fn push_path_segment_parameter(self, segment_parameter: P) -> Self::NewPathParameters;

    /// Undo `push_path_segment_parameter`.
    fn pop_path_segment_parameter_from_output(output: Self::NewPathParameters) -> Self;
}

macro_rules! impl_tuple_push_path_segment_parameter {
    ($($($path_parameter:ident)*;)*) => {
        $(
            impl<$($path_parameter,)*> sealed::PushPathSegmentParameterIsSealed for ($($path_parameter,)*) {}

            impl<$($path_parameter,)* P> PushPathSegmentParameter<P> for ($($path_parameter,)*) {
                type NewPathParameters = ($($path_parameter,)* P,);

                #[allow(non_snake_case)]
                fn push_path_segment_parameter(self, segment_parameter: P) -> Self::NewPathParameters {
                    let ($($path_parameter,)*) = self;

                    ($($path_parameter,)* segment_parameter,)
                }

                #[allow(non_snake_case)]
                fn pop_path_segment_parameter_from_output(($($path_parameter,)* _segment_parameter,): Self::NewPathParameters) -> Self {
                    (($($path_parameter,)*))
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
pub struct ParsePathSegment<P>(PhantomData<fn() -> P>);

impl<P> Clone for ParsePathSegment<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for ParsePathSegment<P> {}

impl<P> fmt::Debug for ParsePathSegment<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ParsePathSegment::<{}>", core::any::type_name::<P>())
    }
}

/// Parse a single segment using the implementation of `core::str::FromStr` of `T`.
pub fn parse_path_segment<P: FromStr>() -> ParsePathSegment<P> {
    ParsePathSegment(PhantomData)
}

impl<CurrentPathParameters: PushPathSegmentParameter<P>, P: FromStr>
    PathDescription<CurrentPathParameters> for ParsePathSegment<P>
{
    type NewPathParameters = CurrentPathParameters::NewPathParameters;

    fn parse_and_validate<
        'r,
        T,
        F: FnOnce(Self::NewPathParameters, Path<'r>) -> Result<T, Self::NewPathParameters>,
    >(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        f: F,
    ) -> Result<T, CurrentPathParameters> {
        let Some((segment_parameter, path)) = path.split_first_segment() else {
            return Err(current_path_parameters);
        };

        match segment_parameter
            .try_into_string::<128>()
            .ok()
            .and_then(|segment_parameter| segment_parameter.parse().ok())
        {
            Some(segment_parameter) => f(
                current_path_parameters.push_path_segment_parameter(segment_parameter),
                path,
            )
            .map_err(PushPathSegmentParameter::pop_path_segment_parameter_from_output),
            None => Err(current_path_parameters),
        }
    }
}

impl<CurrentPathParameters, P: PathDescription<CurrentPathParameters>>
    PathDescription<CurrentPathParameters> for (P,)
{
    type NewPathParameters = P::NewPathParameters;

    fn parse_and_validate<
        'r,
        T,
        F: FnOnce(Self::NewPathParameters, Path<'r>) -> Result<T, Self::NewPathParameters>,
    >(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        validate: F,
    ) -> Result<T, CurrentPathParameters> {
        let (path_parameter,) = self;

        path_parameter.parse_and_validate(current_path_parameters, path, validate)
    }
}

macro_rules! impl_tuple_path_description {
    ($($($name:ident)*;)*) => {
        $(
            impl<CurrentPathParameters, P: PathDescription<CurrentPathParameters> $(,$name: Copy + fmt::Debug)*>
                PathDescription<CurrentPathParameters> for (P, $($name,)*)
            where
                ($($name,)*): PathDescription<P::NewPathParameters>,
            {
                type NewPathParameters = <($($name,)*) as PathDescription<P::NewPathParameters>>::NewPathParameters;

                #[allow(non_snake_case)]
                fn parse_and_validate<
                    'r,
                    T,
                    F: FnOnce(Self::NewPathParameters, Path<'r>) -> Result<T, Self::NewPathParameters>,
                >(
                    &self,
                    current_path_parameters: CurrentPathParameters,
                    path: Path<'r>,
                    validate: F,
                ) -> Result<T, CurrentPathParameters> {
                    let &(P, $($name,)*) = self;

                    P.parse_and_validate(
                        current_path_parameters,
                        path,
                        |current_path_parameters, path| ($($name,)*).parse_and_validate(current_path_parameters, path, validate),
                    )
                }
            }
        )*
    };
}

impl_tuple_path_description!(
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

impl<PD, Handler, Fallback> sealed::PathRouterIsSealed for Route<PD, Handler, Fallback> {}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Handler: MethodHandler<State, PD::NewPathParameters>,
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
        match self
            .path_description
            .parse_entire_path(current_path_parameters, path)
        {
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

/// A service which handles requests at a given path.
pub trait MethodHandlerService<State = (), CurrentPathParameters = ()> {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_method_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        method: &str,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

struct MethodHandlerServicePathRouter<PD, Service, Fallback> {
    path_description: PD,
    service: Service,
    fallback: Fallback,
}

impl<PD, Service, Fallback> sealed::PathRouterIsSealed
    for MethodHandlerServicePathRouter<PD, Service, Fallback>
{
}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Service: MethodHandlerService<
            State,
            <PD as PathDescription<CurrentPathParameters>>::NewPathParameters,
        >,
        Fallback: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters>
    for MethodHandlerServicePathRouter<PD, Service, Fallback>
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self
            .path_description
            .parse_entire_path(current_path_parameters, path)
        {
            Ok(path_parameters) => {
                self.service
                    .call_method_handler_service(
                        state,
                        path_parameters,
                        request.parts.method(),
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

struct NestedService<PD, Service, Fallback> {
    path_description: PD,
    service: Service,
    fallback: Fallback,
}

impl<PD, Service, Fallback> sealed::PathRouterIsSealed for NestedService<PD, Service, Fallback> {}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Service: PathRouter<State, PD::NewPathParameters>,
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
        match self
            .path_description
            .parse_path_prefix(current_path_parameters, path)
        {
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
pub trait PathRouterService<State = (), CurrentPathParameters = ()> {
    /// Handle the request and write the response to the provided  [ResponseWriter].
    async fn call_path_router_service<R: Read, W: ResponseWriter<Error = R::Error>>(
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

impl<PD, Service, Fallback> sealed::PathRouterIsSealed
    for PathRouterServicePathRouter<PD, Service, Fallback>
{
}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Service: PathRouterService<State, <PD as PathDescription<CurrentPathParameters>>::NewPathParameters>,
        Fallback: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters>
    for PathRouterServicePathRouter<PD, Service, Fallback>
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self
            .path_description
            .parse_path_prefix(current_path_parameters, path)
        {
            Ok((path_parameters, path)) => {
                self.service
                    .call_path_router_service(
                        state,
                        path_parameters,
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

impl<Service> sealed::PathRouterIsSealed for ServicePathRouter<Service> {}

impl<State, CurrentPathParameters, Service: PathRouterService<State, CurrentPathParameters>>
    PathRouter<State, CurrentPathParameters> for ServicePathRouter<Service>
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
            .call_path_router_service(
                state,
                current_path_parameters,
                path,
                request,
                response_writer,
            )
            .await
    }
}

/// A [PathRouter] which routes requests to a [MethodHandler].
pub struct Router<
    RouterInner: PathRouter<State, CurrentPathParameters>,
    State = (),
    CurrentPathParameters = (),
> {
    pub(crate) router: RouterInner,
    _data: PhantomData<fn(CurrentPathParameters, State)>,
}

impl<
        RouterInner: PathRouter<State, CurrentPathParameters> + Clone,
        State,
        CurrentPathParameters,
    > Clone for Router<RouterInner, State, CurrentPathParameters>
{
    fn clone(&self) -> Self {
        let &Self { ref router, _data } = self;

        Self {
            router: router.clone(),
            _data,
        }
    }
}

impl<
        RouterInner: PathRouter<State, CurrentPathParameters> + Copy,
        State,
        CurrentPathParameters,
    > Copy for Router<RouterInner, State, CurrentPathParameters>
{
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

impl<State, CurrentPathParameters, Service: PathRouterService<State, CurrentPathParameters>>
    Router<ServicePathRouter<Service>, State, CurrentPathParameters>
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
    /// Add another route to the router.
    ///
    /// ```rust
    /// use picoserve::routing::get;
    ///
    /// let app = picoserve::Router::new()
    ///     .route("/", get(async || "Hello World"))
    ///     .route("/server-name", get(async || "My Server"));
    ///
    /// picoserve::doctests_utils::router(app);
    /// ```
    ///
    /// If the [`MethodHandler`] is created in a separate function, it can return `impl MethodHandler`:
    ///
    /// ```rust
    /// use picoserve::routing::get;
    ///
    /// fn server_name_handler() -> impl picoserve::routing::MethodHandler {
    ///     get(async || "My Server")
    /// }
    ///
    /// let app = picoserve::Router::new()
    ///     .route("/", get(async || "Hello World"))
    ///     .route("/server-name", server_name_handler());
    ///
    /// picoserve::doctests_utils::router(app);
    ///
    /// ```
    ///
    /// Note that if the [`MethodHandler`] accepts path parameters, you'll need to explicitly declare their type.
    ///
    /// ```rust
    /// use picoserve::routing::{get, parse_path_segment};
    ///
    /// struct UserId(usize);
    ///
    /// impl core::str::FromStr for UserId {
    ///     type Err = core::num::ParseIntError;
    ///
    ///     fn from_str(s: &str) -> Result<Self, Self::Err> {
    ///         s.parse().map(Self)
    ///     }
    /// }
    ///
    /// struct AppState {}
    ///
    /// // Replace AppState with your state type, or `()` if there is no state.
    /// fn user_name_handler() -> impl picoserve::routing::MethodHandler<AppState, (UserId,)> {
    ///     get(async |user_id: UserId| {})
    /// }
    ///
    /// let app = picoserve::Router::new()
    ///     .route(("/user", parse_path_segment::<UserId>(), "/name"), user_name_handler());
    ///
    /// picoserve::doctests_utils::router_with_state(app);
    /// ```
    ///
    pub fn route<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        handler: impl MethodHandler<State, PD::NewPathParameters>,
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

    /// Add another route to the router, using a [`MethodHandlerService`] to handle the method routing.
    ///
    /// Note that unless you wish to handle unusual HTTP methods, it's typically better to use [`route`](Router::route) with the `*_service` functions, e.g. [`get_service`].
    /// You can handle additional methods by calling the `*_service` methods on the type returned by the `*_service` functions, e.g. [`post_service`](MethodRouter::post_service).
    ///
    /// ```rust
    /// use picoserve::response::IntoResponse;
    ///
    /// struct ShowMethod;
    ///
    /// impl picoserve::routing::MethodHandlerService for ShowMethod {
    ///     async fn call_method_handler_service<
    ///         R: picoserve::io::Read,
    ///         W: picoserve::response::ResponseWriter<Error = R::Error>,
    ///     >(
    ///         &self,
    ///         _state: &(),
    ///         _current_path_parameters: (),
    ///         method: &str,
    ///         request: picoserve::request::Request<'_, R>,
    ///         response_writer: W,
    ///     ) -> Result<picoserve::ResponseSent, W::Error> {
    ///         format_args!("Method: {method}")
    ///             .write_to(request.body_connection.finalize().await?, response_writer)
    ///             .await
    ///     }
    /// }
    ///
    /// ```
    pub fn route_service<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        service: impl MethodHandlerService<State, PD::NewPathParameters>,
    ) -> Router<impl PathRouter<State, CurrentPathParameters>, State, CurrentPathParameters> {
        let Router {
            router: fallback,
            _data,
        } = self;

        Router {
            router: MethodHandlerServicePathRouter {
                path_description,
                service,
                fallback,
            },
            _data,
        }
    }

    /// Nest a [Router] at some path.
    ///
    /// After removing the prefix described by `path_description`, the rest of the path is passed to `router`.
    ///
    /// ```rust
    /// use picoserve::routing::get;
    ///
    /// let app = picoserve::Router::new().nest(
    ///     "/server-info",
    ///     picoserve::Router::new().route("/name", get(async move || "My Server")),
    /// );
    ///
    /// picoserve::doctests_utils::router(app);
    /// ```
    ///
    /// The nested router can also be declared separately.
    ///
    /// ```rust
    /// use picoserve::routing::get;
    ///
    /// fn server_info() -> picoserve::Router<impl picoserve::routing::PathRouter> {
    ///     picoserve::Router::new().route("/name", get(async move || "My Server"))
    /// }
    ///
    /// let app = picoserve::Router::new().nest("/server-info", server_info());
    /// ```
    ///
    /// Note that if the nested [`Router`] inherits path parameters from its parent, you'll need to explicitly declare their type.
    ///
    /// ```rust
    /// use picoserve::routing::{get, parse_path_segment};
    ///
    /// struct UserId(usize);
    ///
    /// impl core::str::FromStr for UserId {
    ///     type Err = core::num::ParseIntError;
    ///
    ///     fn from_str(s: &str) -> Result<Self, Self::Err> {
    ///         s.parse().map(Self)
    ///     }
    /// }
    ///
    /// struct AppState {}
    ///
    /// // Replace AppState with your state type, or `()` if there is no state.
    /// fn user_info() -> picoserve::Router<impl picoserve::routing::PathRouter<AppState, (UserId,)>, AppState, (UserId,)> {
    ///     picoserve::Router::new().route("/name", get(async |user_id: UserId| {}))
    /// }
    ///
    /// let app = picoserve::Router::new().nest(("/user", parse_path_segment::<UserId>()), user_info());
    ///
    /// picoserve::doctests_utils::router_with_state(app);
    /// ```
    pub fn nest<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        router: Router<impl PathRouter<State, PD::NewPathParameters>, State, PD::NewPathParameters>,
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

    /// Nest a [PathRouterService] at some path, like [nest](Self::nest) but accepts an arbitary service.
    pub fn nest_service<PD: PathDescription<CurrentPathParameters>>(
        self,
        path_description: PD,
        service: impl PathRouterService<State, PD::NewPathParameters>,
    ) -> Router<impl PathRouter<State, CurrentPathParameters>, State, CurrentPathParameters> {
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

    /// Create a cheaply `Copy`able [`Router`], borrowing from this [`Router`].
    pub fn shared(
        &self,
    ) -> Router<
        impl PathRouter<State, CurrentPathParameters> + Copy + '_,
        State,
        CurrentPathParameters,
    > {
        struct SharedPathRouter<'a, RouterInner> {
            router: &'a RouterInner,
        }

        impl<RouterInner> Clone for SharedPathRouter<'_, RouterInner> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<RouterInner> Copy for SharedPathRouter<'_, RouterInner> {}

        impl<RouterInner> sealed::PathRouterIsSealed for SharedPathRouter<'_, RouterInner> {}

        impl<
                State,
                CurrentPathParameters,
                RouterInner: PathRouter<State, CurrentPathParameters>,
            > PathRouter<State, CurrentPathParameters> for SharedPathRouter<'_, RouterInner>
        {
            async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
                &self,
                state: &State,
                current_path_parameters: CurrentPathParameters,
                path: Path<'_>,
                request: Request<'_, R>,
                response_writer: W,
            ) -> Result<ResponseSent, W::Error> {
                self.router
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

        let Self { router, _data } = self;

        Router {
            router: SharedPathRouter { router },
            _data: PhantomData,
        }
    }
}

impl<State, CurrentPathParameters, RouterInner: PathRouter<State, CurrentPathParameters>>
    Router<RouterInner, State, CurrentPathParameters>
{
    /// Provide the state for the router. Amongst other forms, the state itself, or a reference to the state, can be passed.
    /// The provided state will be used for all requests that this router receives, thus ignoring any futher incoming state.
    pub fn with_state<NewState>(
        self,
        state: impl core::borrow::Borrow<State>,
    ) -> Router<impl PathRouter<NewState, CurrentPathParameters>, NewState, CurrentPathParameters>
    {
        struct WithState<State, StateRef, RouterInner> {
            _state: PhantomData<fn(&State)>,
            state_ref: StateRef,
            router: RouterInner,
        }

        impl<State, StateRef, RouterInner> sealed::PathRouterIsSealed
            for WithState<State, StateRef, RouterInner>
        {
        }

        impl<
                NewState,
                State,
                StateRef: core::borrow::Borrow<State>,
                CurrentPathParameters,
                RouterInner: PathRouter<State, CurrentPathParameters>,
            > PathRouter<NewState, CurrentPathParameters>
            for WithState<State, StateRef, RouterInner>
        {
            async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
                &self,
                _state: &NewState,
                current_path_parameters: CurrentPathParameters,
                path: Path<'_>,
                request: Request<'_, R>,
                response_writer: W,
            ) -> Result<ResponseSent, W::Error> {
                self.router
                    .call_path_router(
                        self.state_ref.borrow(),
                        current_path_parameters,
                        path,
                        request,
                        response_writer,
                    )
                    .await
            }
        }

        let Self { router, _data } = self;

        Router {
            router: WithState {
                _state: PhantomData,
                state_ref: state,
                router,
            },
            _data: PhantomData,
        }
    }
}

impl<RouterInner: PathRouter> Router<RouterInner> {
    pub async fn handle_request<R: Read<Error = W::Error>, W: ResponseWriter>(
        &self,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.router
            .call_path_router(&(), (), request.parts.path(), request, response_writer)
            .await
    }
}

/// A [PathRouter] which is either the "Left" route or the "Right" route.
///
/// Used by [Router::either_left_route] and [Router::either_right_route] to create config-time conditional [Router]s.
pub enum EitherPathRoute<L, R> {
    Left { router: L },
    Right { router: R },
}

impl<L, R> sealed::PathRouterIsSealed for EitherPathRoute<L, R> {}

impl<
        State,
        CurrentPathParameters,
        Left: PathRouter<State, CurrentPathParameters>,
        Right: PathRouter<State, CurrentPathParameters>,
    > PathRouter<State, CurrentPathParameters> for EitherPathRoute<Left, Right>
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        match self {
            EitherPathRoute::Left { router } => {
                router
                    .call_path_router(
                        state,
                        current_path_parameters,
                        path,
                        request,
                        response_writer,
                    )
                    .await
            }
            EitherPathRoute::Right { router } => {
                router
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

impl<State, CurrentPathParameters, RouterInner: PathRouter<State, CurrentPathParameters>>
    Router<RouterInner, State, CurrentPathParameters>
{
    /// Transforms the [Router] into the "Left" route of a config-time conditional router.
    pub fn either_left_route<Right: PathRouter<State, CurrentPathParameters>>(
        self,
    ) -> Router<EitherPathRoute<RouterInner, Right>, State, CurrentPathParameters> {
        let Self { router, _data } = self;

        Router {
            router: EitherPathRoute::Left { router },
            _data,
        }
    }

    /// Transforms the [Router] into the "Right" route of a config-time conditional router.
    pub fn either_right_route<Left: PathRouter<State, CurrentPathParameters>>(
        self,
    ) -> Router<EitherPathRoute<Left, RouterInner>, State, CurrentPathParameters> {
        let Self { router, _data } = self;

        Router {
            router: EitherPathRoute::Right { router },
            _data,
        }
    }
}
