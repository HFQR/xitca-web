use std::{borrow::Borrow, convert::Infallible, fmt, future::Future, ops::Deref};

use crate::{handler::FromRequest, request::WebRequest};

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

impl<'a, 'r, S, T> FromRequest<'a, WebRequest<'r, S>> for StateRef<'a, T>
where
    T: 'static,
    S: 'static + Borrow<T>,
{
    type Type<'b> = StateRef<'b, T>;
    type Error = Infallible;
    type Future = impl Future<Output = Result<Self, Self::Error>> where WebRequest<'r, S>: 'a;

    #[inline]
    fn from_request(req: &'a WebRequest<'r, S>) -> Self::Future {
        async move { Ok(StateRef(req.state().borrow())) }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use xitca_http::request::Request;
    use xitca_http_codegen::State;

    use crate::{
        dev::{BuildService, Service},
        handler::handler_service,
        request::WebRequest,
        route::get,
        App,
    };

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

    #[tokio::test]
    async fn state_extract() {
        let state = State {
            field1: String::from("state"),
            field2: 996,
        };

        let service = App::with_current_thread_state(state)
            .at("/", get(handler_service(handler)))
            .finish()
            .build(())
            .await
            .ok()
            .unwrap();

        let _ = service.call(Request::default()).await.unwrap();
    }
}
