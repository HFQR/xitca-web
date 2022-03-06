#[macro_export]
macro_rules! router {
    {$(($route: stmt, $factory: ident)),*} => {
        #[allow(non_camel_case_types)]
        enum EnumService<$($factory),*> {
            $(
                $factory($factory),
            ) +
        }

        impl<Req, Res, Err, $($factory),*> ::xitca_service::Service<Req> for EnumService<$($factory),*>
        where
            $(
                $factory: ::xitca_service::Service<Req, Response = Res, Error = Err>,
            ) +
        {
            type Response = Res;
            type Error = Err;
            type Future<'f> where Self: 'f = impl ::core::future::Future<Output = Result<Self::Response, Self::Error>>;

            #[inline]
            fn call(&self, req: Req) -> Self::Future<'_> {
                async move {
                    match self {
                        $(
                            Self::$factory(ref s) => s.call(req).await,
                        ) +
                    }
                }
            }
        }

        #[allow(non_camel_case_types)]
        #[derive(Clone)]
        enum EnumServiceFactory<$($factory: Clone),*> {
            $(
                $factory($factory),
            ) +
        }

        impl<Req, Arg, Res, Err, $($factory),*> ::xitca_service::ServiceFactory<Req, Arg> for EnumServiceFactory<$($factory),*>
        where
            $(
                $factory: ::xitca_service::ServiceFactory<Req, Arg, Response = Res, Error = Err> + Clone,
                $factory::Future: 'static,
            ) +
        {
            type Response = Res;
            type Error = Err;
            type Service = EnumService<$($factory::Service),*>;
            type Future = ::core::pin::Pin<Box<dyn ::core::future::Future<Output = Result<Self::Service, Self::Error>>>>;

            fn new_service(&self, arg: Arg) -> Self::Future {
                match self {
                    $(
                        Self::$factory(ref f) => {
                            let fut = f.new_service(arg);
                            Box::pin(async move { fut.await.map(EnumService::$factory) })
                        },
                    ) +
                }
            }
        }

        struct RouterService<$($factory),*> {
            routes: ::matchit::Node<EnumService<$($factory),*>>,
        }

        struct RouterServiceFactory {}
    };
}

#[cfg(test)]
mod test {
    #[test]
    fn router() {
        struct foo;
        crate::router![
            ("/", foo) 
        ];
    }
}