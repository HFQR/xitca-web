//! type extractor or application state.

use core::{fmt, ops::Deref};

use crate::{context::WebContext, error::Error, handler::FromRequest};

/// borrow trait for extracting typed field from application state
#[diagnostic::on_unimplemented(
    message = "`{T}` can not be borrowed from {Self}",
    label = "{Self} must impl BorrowState trait for borrowing {T} from app state",
    note = "consider add `impl BorrowState<{T}> for {Self}`"
)]
pub trait BorrowState<T>
where
    T: ?Sized,
{
    fn borrow(&self) -> &T;
}

impl<T> BorrowState<T> for T
where
    T: ?Sized,
{
    fn borrow(&self) -> &T {
        self
    }
}

macro_rules! pointer_impl {
    ($t: path) => {
        impl<T> BorrowState<T> for $t
        where
            T: ?Sized,
        {
            fn borrow(&self) -> &T {
                &*self
            }
        }
    };
}

pointer_impl!(std::boxed::Box<T>);
pointer_impl!(std::rc::Rc<T>);
pointer_impl!(std::sync::Arc<T>);

/// App state extractor.
/// S type must be the same with the type passed to App::with_xxx_state(S).
pub struct StateRef<'a, S>(pub &'a S)
where
    S: ?Sized;

impl<S> fmt::Debug for StateRef<'_, S>
where
    S: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateRef({:?})", self.0)
    }
}

impl<S> fmt::Display for StateRef<'_, S>
where
    S: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateRef({})", self.0)
    }
}

impl<S> Deref for StateRef<'_, S> {
    type Target = S;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for StateRef<'a, T>
where
    C: BorrowState<T>,
    T: ?Sized + 'static,
{
    type Type<'b> = StateRef<'b, T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(StateRef(ctx.state().borrow()))
    }
}

/// App state extractor for owned value.
/// S type must be the same with the type passed to App::with_xxx_state(S).
pub struct StateOwn<S>(pub S);

impl<S> fmt::Debug for StateOwn<S>
where
    S: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateOwn({:?})", self.0)
    }
}

impl<S> fmt::Display for StateOwn<S>
where
    S: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateOwn({})", self.0)
    }
}

impl<S> Deref for StateOwn<S> {
    type Target = S;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for StateOwn<T>
where
    C: BorrowState<T>,
    T: Clone,
{
    type Type<'b> = StateOwn<T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(StateOwn(ctx.state().borrow().clone()))
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{App, handler::handler_service, http::WebRequest, route::get, service::Service};

    use super::*;

    #[derive(Clone, Debug)]
    struct State {
        field1: String,
        field2: u32,
        field3: Arc<dyn std::any::Any + Send + Sync>,
    }

    impl BorrowState<String> for State {
        fn borrow(&self) -> &String {
            &self.field1
        }
    }

    impl BorrowState<u32> for State {
        fn borrow(&self) -> &u32 {
            &self.field2
        }
    }

    impl BorrowState<dyn std::any::Any + Send + Sync> for State {
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

    #[test]
    fn state_extract_deref() {
        use std::{any::Any, sync::Arc};

        async fn handler(StateRef(state): StateRef<'_, dyn Any + Send + Sync>) -> String {
            state.downcast_ref::<i32>().unwrap().to_string()
        }

        async fn handler2(StateRef(state): StateRef<'_, i32>) -> String {
            state.to_string()
        }

        let state = Arc::new(996);

        App::new()
            .with_state(state.clone() as Arc<dyn Any + Send + Sync>)
            .at("/", get(handler_service(handler)))
            .at(
                "/scope",
                App::new().with_state(state).at("/", handler_service(handler2)),
            )
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
