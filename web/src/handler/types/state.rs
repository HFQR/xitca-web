//! type extractor or application state.

use core::{borrow::Borrow, fmt, ops::Deref};

use crate::{
    body::BodyStream,
    context::WebContext,
    handler::{error::ExtractError, FromRequest},
};

/// App state extractor.
/// S type must be the same with the type passed to App::with_xxx_state(S).
pub struct StateRef<'a, S>(pub &'a S);

impl<S: fmt::Debug> fmt::Debug for StateRef<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateRef({:?})", self.0)
    }
}

impl<S: fmt::Display> fmt::Display for StateRef<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateRef({})", self.0)
    }
}

impl<S> Deref for StateRef<'_, S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for StateRef<'a, T>
where
    C: Borrow<T>,
    B: BodyStream,
    T: 'static,
{
    type Type<'b> = StateRef<'b, T>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(StateRef(req.state().borrow()))
    }
}

/// App state extractor for owned value.
/// S type must be the same with the type passed to App::with_xxx_state(S).
pub struct StateOwn<S>(pub S);

impl<S: fmt::Debug> fmt::Debug for StateOwn<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateOwn({:?})", self.0)
    }
}

impl<S: fmt::Display> fmt::Display for StateOwn<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateOwn({})", self.0)
    }
}

impl<S> Deref for StateOwn<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for StateOwn<T>
where
    C: Borrow<T>,
    B: BodyStream,
    T: Clone,
{
    type Type<'b> = StateOwn<T>;
    type Error = ExtractError<B::Error>;

    #[inline]
    async fn from_request(req: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(StateOwn(req.state().borrow().clone()))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_codegen::State;
    use xitca_http::Request;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{dev::service::Service, handler::handler_service, route::get, App};

    #[derive(State, Clone, Debug, Eq, PartialEq)]
    struct State {
        #[borrow]
        field1: String,
        #[borrow]
        field2: u32,
    }

    async fn handler(
        StateRef(state): StateRef<'_, String>,
        StateRef(state2): StateRef<'_, u32>,
        StateRef(state3): StateRef<'_, State>,
        req: &WebContext<'_, State>,
    ) -> String {
        assert_eq!("state", state);
        assert_eq!(&996, state2);
        assert_eq!(state, req.state().field1.as_str());
        assert_eq!(state3, req.state());
        state.to_string()
    }

    #[test]
    fn state_extract() {
        let state = State {
            field1: String::from("state"),
            field2: 996,
        };

        App::with_state(state)
            .at("/", get(handler_service(handler)))
            .finish()
            .call(())
            .now_or_panic()
            .ok()
            .unwrap()
            .call(Request::default())
            .now_or_panic()
            .unwrap();
    }
}
