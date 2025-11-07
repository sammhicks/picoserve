use crate::{
    io::Read,
    request::{Path, Request, RequestParts},
    ResponseSent,
};

use super::{sealed::Sealed, MethodHandler, PathRouter, ResponseWriter};

/// The remainer of a middleware stack, including the handler.
pub trait Next<'a, R: Read + 'a, State, PathParameters>: Sealed + Sized {
    /// Run the next layer, writing the final response to the [ResponseWriter]
    async fn run<W: ResponseWriter<Error = R::Error>>(
        self,
        state: &State,
        path_parameters: PathParameters,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;

    fn into_request(self) -> Request<'a, R>;

    async fn into_connection(
        self,
    ) -> Result<crate::response::Connection<'a, impl Read<Error = R::Error>>, R::Error> {
        self.into_request().body_connection.finalize().await
    }
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
    /// The state passed to the next layer
    type NextState;
    /// The parameters passed to the next layer
    type NextPathParameters;

    /// Call the current layer, passing inner layers
    async fn call_layer<
        'a,
        R: Read + 'a,
        NextLayer: Next<'a, R, Self::NextState, Self::NextPathParameters>,
        W: ResponseWriter<Error = R::Error>,
    >(
        &self,
        next: NextLayer,
        state: &State,
        path_parameters: PathParameters,
        request_parts: RequestParts<'_>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

struct NextMethodRouterLayer<'a, R: Read, N> {
    next: &'a N,
    request: Request<'a, R>,
}

impl<R: Read, N> Sealed for NextMethodRouterLayer<'_, R, N> {}

impl<'a, R: Read, State, PathParameters, N: MethodHandler<State, PathParameters>>
    Next<'a, R, State, PathParameters> for NextMethodRouterLayer<'a, R, N>
{
    async fn run<W: ResponseWriter<Error = R::Error>>(
        self,
        state: &State,
        path_parameters: PathParameters,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.next
            .call_method_handler(state, path_parameters, self.request, response_writer)
            .await
    }

    fn into_request(self) -> Request<'a, R> {
        self.request
    }
}

pub(crate) struct MethodRouterLayer<L, I> {
    pub(crate) layer: L,
    pub(crate) inner: I,
}

impl<L, I> Sealed for MethodRouterLayer<L, I> {}

impl<
        L: Layer<State, PathParameters>,
        I: MethodHandler<L::NextState, L::NextPathParameters>,
        State,
        PathParameters,
    > MethodHandler<State, PathParameters> for MethodRouterLayer<L, I>
{
    async fn call_method_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        path_parameters: PathParameters,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let request_parts = request.parts;

        self.layer
            .call_layer(
                NextMethodRouterLayer {
                    next: &self.inner,
                    request,
                },
                state,
                path_parameters,
                request_parts,
                response_writer,
            )
            .await
    }
}

struct NextPathRouterLayer<'a, R: Read, N> {
    next: &'a N,
    path: Path<'a>,
    request: Request<'a, R>,
}

impl<R: Read, N> Sealed for NextPathRouterLayer<'_, R, N> {}

impl<'a, R: Read, State, CurrentPathParameters, N: PathRouter<State, CurrentPathParameters>>
    Next<'a, R, State, CurrentPathParameters> for NextPathRouterLayer<'a, R, N>
{
    async fn run<W: ResponseWriter<Error = R::Error>>(
        self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.next
            .call_path_router(
                state,
                current_path_parameters,
                self.path,
                self.request,
                response_writer,
            )
            .await
    }

    fn into_request(self) -> Request<'a, R> {
        self.request
    }
}

pub(crate) struct PathRouterLayer<L, I> {
    pub(crate) layer: L,
    pub(crate) inner: I,
}

impl<L, I> Sealed for PathRouterLayer<L, I> {}

impl<
        L: Layer<State, CurrentPathParameters>,
        I: PathRouter<L::NextState, L::NextPathParameters>,
        State,
        CurrentPathParameters,
    > PathRouter<State, CurrentPathParameters> for PathRouterLayer<L, I>
{
    async fn call_path_router<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        state: &State,
        current_path_parameters: CurrentPathParameters,
        path: Path<'_>,
        request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let request_parts = request.parts;

        self.layer
            .call_layer(
                NextPathRouterLayer {
                    next: &self.inner,
                    path,
                    request,
                },
                state,
                current_path_parameters,
                request_parts,
                response_writer,
            )
            .await
    }
}
