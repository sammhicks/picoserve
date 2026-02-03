use crate::{
    io::{Read, Write},
    ResponseSent,
};

use super::{Connection, Content, IntoResponse, ResponseWriter};

/// [`Content`] which uses the State to calculate its properties
pub trait ContentUsingState<State> {
    fn content_type(&self, state: &State) -> &'static str;

    fn content_length(&self, state: &State) -> usize;

    async fn write_content<W: Write>(self, state: &State, writer: W) -> Result<(), W::Error>;

    /// Convert into a type which implements [`Content`] and thus can be passed into [`Response::new`](super::Response::new),
    ///  or as the last field in a tuple.
    fn using_state(self, state: &State) -> ContentUsingStateWithState<'_, State, Self>
    where
        Self: Sized,
    {
        ContentUsingStateWithState {
            content: self,
            state,
        }
    }
}

/// A [`Content`] which passes the State to the [`ContentUsingState`].
pub struct ContentUsingStateWithState<'s, State, C: ContentUsingState<State>> {
    content: C,
    state: &'s State,
}

impl<State, C: ContentUsingState<State>> Content for ContentUsingStateWithState<'_, State, C> {
    fn content_type(&self) -> &'static str {
        self.content.content_type(self.state)
    }

    fn content_length(&self) -> usize {
        self.content.content_length(self.state)
    }

    async fn write_content<W: Write>(self, writer: W) -> Result<(), W::Error> {
        self.content.write_content(self.state, writer).await
    }
}

/// Trait for generating responses which use the State when writing themselves to the socket.
///
/// Types that implement `IntoResponseWithState` can be returned from handlers.
/// [`IntoResponse`] should be preferred, with [`IntoResponseWithState`] used if copying out the appropriate part of State is costly.
pub trait IntoResponseWithState<State>: Sized {
    /// Write the generated response into the given [`ResponseWriter`].
    async fn write_to_with_state<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        state: &State,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error>;
}

impl<T: IntoResponse, State> IntoResponseWithState<State> for T {
    async fn write_to_with_state<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        _state: &State,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        self.write_to(connection, response_writer).await
    }
}

/// A response which also updates the state. Returned by [`WithStateUpdate::with_state_update`]
pub struct IntoResponseWithStateUpdate<
    State,
    T: IntoResponseWithState<State>,
    F: AsyncFnOnce(&State),
> {
    response: T,
    state_update: F,
    _state: core::marker::PhantomData<fn(&State)>,
}

impl<State, T: IntoResponseWithState<State>, F: AsyncFnOnce(&State)> IntoResponseWithState<State>
    for IntoResponseWithStateUpdate<State, T, F>
{
    async fn write_to_with_state<R: Read, W: ResponseWriter<Error = R::Error>>(
        self,
        state: &State,
        connection: Connection<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let Self {
            response,
            state_update,
            _state: core::marker::PhantomData,
        } = self;

        state_update(state).await;

        response
            .write_to_with_state(state, connection, response_writer)
            .await
    }
}

/// An extension trait for updating the state as part of writing the response.
///
/// Allows for easy state updates using data produced by the handler.
pub trait WithStateUpdate<State>: IntoResponseWithState<State> {
    fn with_state_update<F: AsyncFnOnce(&State)>(
        self,
        state_update: F,
    ) -> IntoResponseWithStateUpdate<State, Self, F> {
        IntoResponseWithStateUpdate {
            response: self,
            state_update,
            _state: core::marker::PhantomData,
        }
    }
}

impl<State, T: IntoResponseWithState<State>> WithStateUpdate<State> for T {}
