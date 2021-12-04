pub use xitca_http::util::service::HandlerService;

#[cfg(test)]
mod test {
    use crate::extract::State;
    use crate::request::WebRequest;

    use super::HandlerService;

    use xitca_service::{Service, ServiceFactory, ServiceFactoryExt};

    async fn handler(state: State<'_, String>) -> String {
        assert_eq!("123", state.as_str());
        state.to_string()
    }

    #[tokio::test]
    async fn handler_service() {
        let service = HandlerService::new(handler)
            .transform_fn(|s, req| async move { s.call(req).await })
            .new_service(())
            .await
            .ok()
            .unwrap();

        let data = String::from("123");

        let mut req = WebRequest::with_state(&data);

        let res = service.call(&mut req).await.ok().unwrap();

        assert_eq!(res.status().as_u16(), 200);
    }
}
