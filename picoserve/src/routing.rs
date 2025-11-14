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
    pub trait RequestHandlerFunctionIsSealed<State, PathParameters, Shape> {}

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
pub trait RequestHandlerFunction<State, PathParameters, Shape>:
    sealed::RequestHandlerFunctionIsSealed<State, PathParameters, Shape>
{
    /// Call the handler function and write the response to the [ResponseWriter].
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
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
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
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
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
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
    async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
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
                async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
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
                async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
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
                async fn call_handler_func<R: Read, W: ResponseWriter<Error = R::Error>>(
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

struct HandlerFunctionRequestHandler<T, Handler> {
    phantom_data: PhantomData<fn(&T)>,
    handler: Handler,
}

impl<T, Handler> sealed::RequestHandlerIsSealed for HandlerFunctionRequestHandler<T, Handler> {}

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

/// A [MethodHandler] which routes requests to the appropriate [RequestHandler] based on the method.
///
/// Automatically handled the `HEAD` method by calling the `GET` handler and returning an empty body.
pub struct MethodRouter<GET, POST, PUT, DELETE, OPTIONS> {
    get: GET,
    post: POST,
    put: PUT,
    delete: DELETE,
    options: OPTIONS,
}

impl<GET, POST, PUT, DELETE, OPTIONS> sealed::MethodHandlerIsSealed
    for MethodRouter<GET, POST, PUT, DELETE, OPTIONS>
{
}

/// Route `GET` requests to the given [handler](RequestHandlerFunction).
pub fn get<State, PathParameters, T, Handler: RequestHandlerFunction<State, PathParameters, T>>(
    handler: Handler,
) -> MethodRouter<
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
> {
    MethodRouter {
        get: HandlerFunctionRequestHandler::new(handler),
        post: MethodNotAllowed,
        put: MethodNotAllowed,
        delete: MethodNotAllowed,
        options: MethodNotAllowed,
    }
}

/// Route `GET` requests to the given [service](RequestHandlerService).
pub fn get_service<State, PathParameters>(
    service: impl RequestHandlerService<State, PathParameters>,
) -> MethodRouter<
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
> {
    MethodRouter {
        get: RequestHandlerServiceRequestHandler { service },
        post: MethodNotAllowed,
        put: MethodNotAllowed,
        delete: MethodNotAllowed,
        options: MethodNotAllowed,
    }
}

/// Route `POST` requests to the given [handler](RequestHandlerFunction).
pub fn post<State, PathParameters, T, Handler: RequestHandlerFunction<State, PathParameters, T>>(
    handler: Handler,
) -> MethodRouter<
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: HandlerFunctionRequestHandler::new(handler),
        put: MethodNotAllowed,
        delete: MethodNotAllowed,
        options: MethodNotAllowed,
    }
}

/// Route `POST` requests to the given [service](RequestHandlerService).
pub fn post_service<State, PathParameters>(
    service: impl RequestHandlerService<State, PathParameters>,
) -> MethodRouter<
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: RequestHandlerServiceRequestHandler { service },
        put: MethodNotAllowed,
        delete: MethodNotAllowed,
        options: MethodNotAllowed,
    }
}

/// Route `PUT` requests to the given [handler](RequestHandlerFunction).
pub fn put<State, PathParameters, T, Handler: RequestHandlerFunction<State, PathParameters, T>>(
    handler: Handler,
) -> MethodRouter<
    MethodNotAllowed,
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
    MethodNotAllowed,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: MethodNotAllowed,
        put: HandlerFunctionRequestHandler::new(handler),
        delete: MethodNotAllowed,
        options: MethodNotAllowed,
    }
}

/// Route `PUT` requests to the given [service](RequestHandlerService).
pub fn put_service<State, PathParameters>(
    service: impl RequestHandlerService<State, PathParameters>,
) -> MethodRouter<
    MethodNotAllowed,
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
    MethodNotAllowed,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: MethodNotAllowed,
        put: RequestHandlerServiceRequestHandler { service },
        delete: MethodNotAllowed,
        options: MethodNotAllowed,
    }
}

/// Route `DELETE` requests to the given [handler](RequestHandlerFunction).
pub fn delete<
    State,
    PathParameters,
    T,
    Handler: RequestHandlerFunction<State, PathParameters, T>,
>(
    handler: Handler,
) -> MethodRouter<
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: MethodNotAllowed,
        put: MethodNotAllowed,
        delete: HandlerFunctionRequestHandler::new(handler),
        options: MethodNotAllowed,
    }
}

/// Route `DELETE` requests to the given [service](RequestHandlerService).
pub fn delete_service<State, PathParameters>(
    service: impl RequestHandlerService<State, PathParameters>,
) -> MethodRouter<
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
    MethodNotAllowed,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: MethodNotAllowed,
        put: MethodNotAllowed,
        delete: RequestHandlerServiceRequestHandler { service },
        options: MethodNotAllowed,
    }
}

/// Route `OPTIONS` requests to the given [handler](RequestHandlerFunction).
pub fn options<
    State,
    PathParameters,
    T,
    Handler: RequestHandlerFunction<State, PathParameters, T>,
>(
    handler: Handler,
) -> MethodRouter<
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: MethodNotAllowed,
        put: MethodNotAllowed,
        delete: MethodNotAllowed,
        options: HandlerFunctionRequestHandler::new(handler),
    }
}

/// Route `OPTIONS` requests to the given [service](RequestHandlerService).
pub fn options_service<State, PathParameters>(
    service: impl RequestHandlerService<State, PathParameters>,
) -> MethodRouter<
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    MethodNotAllowed,
    impl RequestHandler<State, PathParameters>,
> {
    MethodRouter {
        get: MethodNotAllowed,
        post: MethodNotAllowed,
        put: MethodNotAllowed,
        delete: MethodNotAllowed,
        options: RequestHandlerServiceRequestHandler { service },
    }
}

impl<POST, PUT, DELETE, OPTIONS> MethodRouter<MethodNotAllowed, POST, PUT, DELETE, OPTIONS> {
    /// Chain an additional [handler](RequestHandlerFunction) that will only accept `GET` requests.
    pub fn get<
        State,
        PathParameters,
        T,
        Handler: RequestHandlerFunction<State, PathParameters, T>,
    >(
        self,
        handler: Handler,
    ) -> MethodRouter<impl RequestHandler<State, PathParameters>, POST, PUT, DELETE, OPTIONS> {
        let MethodRouter {
            get: MethodNotAllowed,
            post,
            put,
            delete,
            options,
        } = self;

        MethodRouter {
            get: HandlerFunctionRequestHandler::new(handler),
            post,
            put,
            delete,
            options,
        }
    }

    /// Chain an additional [service](RequestHandlerService) that will only accept `GET` requests.
    pub fn get_service<State, PathParameters>(
        self,
        service: impl RequestHandlerService<State, PathParameters>,
    ) -> MethodRouter<impl RequestHandler<State, PathParameters>, POST, PUT, DELETE, OPTIONS> {
        let MethodRouter {
            get: MethodNotAllowed,
            post,
            put,
            delete,
            options,
        } = self;

        MethodRouter {
            get: RequestHandlerServiceRequestHandler { service },
            post,
            put,
            delete,
            options,
        }
    }
}

impl<GET, PUT, DELETE, OPTIONS> MethodRouter<GET, MethodNotAllowed, PUT, DELETE, OPTIONS> {
    /// Chain an additional [handler](RequestHandlerFunction) that will only accept `POST` requests.
    pub fn post<
        State,
        PathParameters,
        T,
        Handler: RequestHandlerFunction<State, PathParameters, T>,
    >(
        self,
        handler: Handler,
    ) -> MethodRouter<GET, impl RequestHandler<State, PathParameters>, PUT, DELETE, OPTIONS> {
        let MethodRouter {
            get,
            post: MethodNotAllowed,
            put,
            delete,
            options,
        } = self;

        MethodRouter {
            get,
            post: HandlerFunctionRequestHandler::new(handler),
            put,
            delete,
            options,
        }
    }

    /// Chain an additional [service](RequestHandlerService) that will only accept `POST` requests.
    pub fn post_service<State, PathParameters>(
        self,
        service: impl RequestHandlerService<State, PathParameters>,
    ) -> MethodRouter<GET, impl RequestHandler<State, PathParameters>, PUT, DELETE, OPTIONS> {
        let MethodRouter {
            get,
            post: MethodNotAllowed,
            put,
            delete,
            options,
        } = self;

        MethodRouter {
            get,
            post: RequestHandlerServiceRequestHandler { service },
            put,
            delete,
            options,
        }
    }
}

impl<GET, POST, DELETE, OPTIONS> MethodRouter<GET, POST, MethodNotAllowed, DELETE, OPTIONS> {
    /// Chain an additional [handler](RequestHandlerFunction) that will only accept `PUT` requests.
    pub fn put<
        State,
        PathParameters,
        T,
        Handler: RequestHandlerFunction<State, PathParameters, T>,
    >(
        self,
        handler: Handler,
    ) -> MethodRouter<GET, POST, impl RequestHandler<State, PathParameters>, DELETE, OPTIONS> {
        let MethodRouter {
            get,
            post,
            put: MethodNotAllowed,
            delete,
            options,
        } = self;

        MethodRouter {
            get,
            post,
            put: HandlerFunctionRequestHandler::new(handler),
            delete,
            options,
        }
    }

    /// Chain an additional [service](RequestHandlerService) that will only accept `PUT` requests.
    pub fn put_service<State, PathParameters>(
        self,
        service: impl RequestHandlerService<State, PathParameters>,
    ) -> MethodRouter<GET, POST, impl RequestHandler<State, PathParameters>, DELETE, OPTIONS> {
        let MethodRouter {
            get,
            post,
            put: MethodNotAllowed,
            delete,
            options,
        } = self;

        MethodRouter {
            get,
            post,
            put: RequestHandlerServiceRequestHandler { service },
            delete,
            options,
        }
    }
}

impl<GET, POST, PUT, OPTIONS> MethodRouter<GET, POST, PUT, MethodNotAllowed, OPTIONS> {
    /// Chain an additional [handler](RequestHandlerFunction) that will only accept `DELETE` requests.
    pub fn delete<
        State,
        PathParameters,
        T,
        Handler: RequestHandlerFunction<State, PathParameters, T>,
    >(
        self,
        handler: Handler,
    ) -> MethodRouter<GET, POST, PUT, impl RequestHandler<State, PathParameters>, OPTIONS> {
        let MethodRouter {
            get,
            post,
            put,
            delete: MethodNotAllowed,
            options,
        } = self;

        MethodRouter {
            get,
            post,
            put,
            delete: HandlerFunctionRequestHandler::new(handler),
            options,
        }
    }

    /// Chain an additional [service](RequestHandlerService) that will only accept `DELETE` requests.
    pub fn delete_service<State, PathParameters>(
        self,
        service: impl RequestHandlerService<State, PathParameters>,
    ) -> MethodRouter<GET, POST, PUT, impl RequestHandler<State, PathParameters>, OPTIONS> {
        let MethodRouter {
            get,
            post,
            put,
            delete: MethodNotAllowed,
            options,
        } = self;

        MethodRouter {
            get,
            post,
            put,
            delete: RequestHandlerServiceRequestHandler { service },
            options,
        }
    }
}

impl<GET, POST, PUT, DELETE> MethodRouter<GET, POST, PUT, DELETE, MethodNotAllowed> {
    /// Chain an additional [handler](RequestHandlerFunction) that will only accept `OPTIONS` requests.
    pub fn options<
        State,
        PathParameters,
        T,
        Handler: RequestHandlerFunction<State, PathParameters, T>,
    >(
        self,
        handler: Handler,
    ) -> MethodRouter<GET, POST, PUT, DELETE, impl RequestHandler<State, PathParameters>> {
        let MethodRouter {
            get,
            post,
            put,
            delete,
            options: MethodNotAllowed,
        } = self;

        MethodRouter {
            get,
            post,
            put,
            delete,
            options: HandlerFunctionRequestHandler::new(handler),
        }
    }

    /// Chain an additional [service](RequestHandlerService) that will only accept `OPTIONS` requests.
    pub fn options_service<State, PathParameters>(
        self,
        service: impl RequestHandlerService<State, PathParameters>,
    ) -> MethodRouter<GET, POST, PUT, DELETE, impl RequestHandler<State, PathParameters>> {
        let MethodRouter {
            get,
            post,
            put,
            delete,
            options: MethodNotAllowed,
        } = self;

        MethodRouter {
            get,
            post,
            put,
            delete,
            options: RequestHandlerServiceRequestHandler { service },
        }
    }
}

impl<GET, POST, PUT, DELETE, OPTIONS> MethodRouter<GET, POST, PUT, DELETE, OPTIONS> {
    /// Add a [Layer] to all routes in the router
    pub fn layer<State, PathParameters, L: Layer<State, PathParameters>>(
        self,
        layer: L,
    ) -> impl MethodHandler<State, PathParameters>
    where
        GET: RequestHandler<L::NextState, L::NextPathParameters>,
        POST: RequestHandler<L::NextState, L::NextPathParameters>,
        PUT: RequestHandler<L::NextState, L::NextPathParameters>,
        DELETE: RequestHandler<L::NextState, L::NextPathParameters>,
        OPTIONS: RequestHandler<L::NextState, L::NextPathParameters>,
    {
        layer::MethodRouterLayer { layer, inner: self }
    }
}

impl<
        State,
        PathParameters,
        GET: RequestHandler<State, PathParameters>,
        POST: RequestHandler<State, PathParameters>,
        PUT: RequestHandler<State, PathParameters>,
        DELETE: RequestHandler<State, PathParameters>,
        OPTIONS: RequestHandler<State, PathParameters>,
    > MethodHandler<State, PathParameters> for MethodRouter<GET, POST, PUT, DELETE, OPTIONS>
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
            "PUT" => {
                self.put
                    .call_request_handler(state, path_parameters, request, response_writer)
                    .await
            }
            "DELETE" => {
                self.delete
                    .call_request_handler(state, path_parameters, request, response_writer)
                    .await
            }
            "OPTIONS" => {
                self.options
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
    /// The output of the parsed path description. Must implement [PushPathSegmentParameter] if not the final path description.
    type Output;

    /// Parse the path and then call the validation function.
    fn parse_and_validate<'r, T, F: FnOnce(Self::Output, Path<'r>) -> Result<T, Self::Output>>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
        validate: F,
    ) -> Result<T, CurrentPathParameters>;

    fn parse_prefix<'r>(
        &self,
        current_path_parameters: CurrentPathParameters,
        path: Path<'r>,
    ) -> Result<(Self::Output, Path<'r>), CurrentPathParameters> {
        self.parse_and_validate(current_path_parameters, path, |path_parameters, path| {
            if path.0.is_empty() {
                Err(path_parameters)
            } else {
                Ok((path_parameters, path))
            }
        })
    }
}

impl<CurrentPathParameters> PathDescription<CurrentPathParameters> for &str {
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
pub trait PushPathSegmentParameter<P>: sealed::PushPathSegmentParameterIsSealed + Sized {
    /// The concatenation of the current value and the new value.
    type Output;

    fn push_path_segment_parameter(self, segment_parameter: P) -> Self::Output;

    fn pop_path_segment_parameter_from_output(output: Self::Output) -> Self;
}

macro_rules! impl_tuple_push_path_segment_parameter {
    ($($($path_parameter:ident)*;)*) => {
        $(
            impl<$($path_parameter,)*> sealed::PushPathSegmentParameterIsSealed for ($($path_parameter,)*) {}

            impl<$($path_parameter,)* P> PushPathSegmentParameter<P> for ($($path_parameter,)*) {
                type Output = ($($path_parameter,)* P,);

                #[allow(non_snake_case)]
                fn push_path_segment_parameter(self, segment_parameter: P) -> Self::Output {
                    let ($($path_parameter,)*) = self;

                    ($($path_parameter,)* segment_parameter,)
                }

                #[allow(non_snake_case)]
                fn pop_path_segment_parameter_from_output(($($path_parameter,)* _segment_parameter,): Self::Output) -> Self {
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
            impl<CurrentPathParameters, P: PathDescription<CurrentPathParameters> $(,$name: Copy + fmt::Debug)*>
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

impl<PD, Handler, Fallback> sealed::PathRouterIsSealed for Route<PD, Handler, Fallback> {}

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

impl<PD, Service, Fallback> sealed::PathRouterIsSealed for NestedService<PD, Service, Fallback> {}

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
        match self
            .path_description
            .parse_prefix(current_path_parameters, path)
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

impl<PD, Service, Fallback> sealed::PathRouterIsSealed
    for PathRouterServicePathRouter<PD, Service, Fallback>
{
}

impl<
        State,
        CurrentPathParameters,
        PD: PathDescription<CurrentPathParameters>,
        Service: PathRouterService<State, <PD as PathDescription<CurrentPathParameters>>::Output>,
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
            .parse_prefix(current_path_parameters, path)
        {
            Ok((path_parameters, path)) => {
                self.service
                    .call_request_handler_service(
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
            .call_request_handler_service(
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
        router: Router<impl PathRouter<State, PD::Output>, State, PD::Output>,
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
        service: impl PathRouterService<State, PD::Output>,
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
