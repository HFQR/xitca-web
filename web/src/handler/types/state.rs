use std::{borrow::Borrow, fmt, future::Future, ops::Deref};

use crate::{
    handler::{error::ExtractError, FromRequest},
    request::WebRequest,
    stream::WebStream,
};

/// App state extractor.
/// S type must be the same with the type passed to App::with_xxx_state(<S>).
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

impl<'a, 'r, C, B, T> FromRequest<'a, WebRequest<'r, C, B>> for StateRef<'a, T>
where
    C: Borrow<T>,
    B: WebStream,
    T: 'static,
{
    type Type<'b> = StateRef<'b, T>;
    type Error = ExtractError<B::Error>;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, C, B>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, C, B>) -> Self::Future {
        async { Ok(StateRef(req.state().borrow())) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_codegen::State;
    use xitca_http::Request;
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{dev::service::Service, handler::handler_service, request::WebRequest, route::get, App};

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
        req: &WebRequest<'_, State>,
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

        App::with_current_thread_state(state)
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
