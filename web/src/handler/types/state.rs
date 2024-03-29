//! type extractor or application state.

use core::{borrow::Borrow, fmt, ops::Deref};

use crate::{context::WebContext, error::Error, handler::FromRequest};

/// App state extractor.
/// S type must be the same with the type passed to App::with_xxx_state(S).
pub struct StateRef<'a, S>(pub &'a S)
where
    S: ?Sized;

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
    T: ?Sized + 'static,
{
    type Type<'b> = StateRef<'b, T>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(StateRef(ctx.state().borrow()))
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
    T: Clone,
{
    type Type<'b> = StateOwn<T>;
    type Error = Error<C>;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(StateOwn(ctx.state().borrow().clone()))
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use xitca_codegen::State;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{handler::handler_service, http::WebRequest, route::get, service::Service, App};

    use super::*;

    #[derive(State, Clone, Debug)]
    struct State {
        #[borrow]
        field1: String,
        #[borrow]
        field2: u32,
        field3: Arc<dyn std::any::Any + Send + Sync>,
    }

    impl Borrow<dyn std::any::Any + Send + Sync> for State {
        fn borrow(&self) -> &(dyn std::any::Any + Send + Sync) {
            &*self.field3
        }
    }

    async fn handler(
        StateRef(state): StateRef<'_, String>,
        StateRef(state2): StateRef<'_, u32>,
        StateRef(state3): StateRef<'_, State>,
        StateRef(state4): StateRef<'_, dyn std::any::Any + Send + Sync>,
        ctx: &WebContext<'_, State>,
    ) -> String {
        assert_eq!("state", state);
        assert_eq!(&996, state2);
        assert_eq!(state, ctx.state().field1.as_str());
        assert_eq!(state3.field1, ctx.state().field1);
        assert!(state4.downcast_ref::<String>().is_some());
        state.to_string()
    }

    #[test]
    fn state_extract() {
        let state = State {
            field1: String::from("state"),
            field2: 996,
            field3: Arc::new(String::new()),
        };

        App::new()
            .with_state(state)
            .at("/", get(handler_service(handler)))
            .finish()
            .call(())
            .now_or_panic()
            .ok()
            .unwrap()
            .call(WebRequest::default())
            .now_or_panic()
            .unwrap();
    }
}
