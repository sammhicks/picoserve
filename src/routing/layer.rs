use embedded_io_async::{Read, Write};

use crate::{
    request::{Path, Request},
    ResponseSent,
};

use super::{MethodHandler, PathRouter, ResponseWriter};

/// The remainer of a middleware stack, including the handler.
pub trait Next<State, PathParameters> {
    async fn run<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        self,
        state: &State,
        path_parameters: PathParameters,
        body_reader: R,
        write: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error>;
}

/// A middleware "layer", which can be used to inspect requests and transform responses.
///
/// Layers can be used to:
/// + inspect the request before it is passed to the inner handler
/// + send a different state to the inner handler than the state passed to the layer
/// + send different path parameters to the inner handler than the path parameters passed to the layer
/// + send a response instead of passing the request to the inner handler
/// + send a different response than the one returned by the inner handler
/// + and more...
///
/// To modify the response, create a struct that implements [ResponseWriter] and wraps `response_writer`,
/// and pass an instance of that struct to `next`
pub trait Layer<State, PathParameters> {
    /// The state passed to the
    type NextState;
    type NextPathParameters;

    async fn call_layer<
        NextLayer: Next<Self::NextState, Self::NextPathParameters>,
        R: Read,
        WW: Write<Error = R::Error>,
        W: ResponseWriter,
    >(
        &self,
        next: NextLayer,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        write: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error>;
}

struct NextMethodRouterLayer<'a, N> {
    next: &'a N,
    request: Request<'a>,
}

impl<'a, State, PathParameters, N: MethodHandler<State, PathParameters>> Next<State, PathParameters>
    for NextMethodRouterLayer<'a, N>
{
    async fn run<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        self,
        state: &State,
        path_parameters: PathParameters,
        body_reader: R,
        write: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        self.next
            .call_method_handler(
                state,
                path_parameters,
                self.request,
                body_reader,
                write,
                response_writer,
            )
            .await
    }
}

pub(crate) struct MethodRouterLayer<L, I> {
    pub(crate) layer: L,
    pub(crate) inner: I,
}

impl<
        L: Layer<State, PathParameters>,
        I: MethodHandler<L::NextState, L::NextPathParameters>,
        State,
        PathParameters,
    > MethodHandler<State, PathParameters> for MethodRouterLayer<L, I>
{
    async fn call_method_handler<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_>,
        body_reader: R,
        write: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        self.layer
            .call_layer(
                NextMethodRouterLayer {
                    next: &self.inner,
                    request,
                },
                state,
                path_parameters,
                request,
                body_reader,
                write,
                response_writer,
            )
            .await
    }
}

struct NextPathRouterLayer<'a, N> {
    next: &'a N,
    path: Path<'a>,
    request: Request<'a>,
}

impl<'a, State, CurrentPathParameters, N: PathRouter<State, CurrentPathParameters>>
    Next<State, CurrentPathParameters> for NextPathRouterLayer<'a, N>
{
    async fn run<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        body_reader: R,
        write: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        self.next
            .call_path_router(
                state,
                current_path_parameters,
                self.path,
                self.request,
                body_reader,
                write,
                response_writer,
            )
            .await
    }
}

pub(crate) struct PathRouterLayer<L, I> {
    pub(crate) layer: L,
    pub(crate) inner: I,
}

impl<
        L: Layer<State, CurrentPathParameters>,
        I: PathRouter<L::NextState, L::NextPathParameters>,
        State,
        CurrentPathParameters,
    > PathRouter<State, CurrentPathParameters> for PathRouterLayer<L, I>
{
    async fn call_path_router<R: Read, WW: Write<Error = R::Error>, W: ResponseWriter>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_>,
        body_reader: R,
        write: WW,
        response_writer: W,
    ) -> Result<ResponseSent, R::Error> {
        self.layer
            .call_layer(
                NextPathRouterLayer {
                    next: &self.inner,
                    path,
                    request,
                },
                state,
                current_path_parameters,
                request,
                body_reader,
                write,
                response_writer,
            )
            .await
    }
}
