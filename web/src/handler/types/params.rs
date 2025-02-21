//! type extractor for uri params.

use core::{marker::PhantomData, ops::Deref};

use serde::{
    Deserialize,
    de::{self, Deserializer, Error as DeError, Visitor},
    forward_to_deserialize_any,
};

use xitca_http::util::service::router;

use crate::{context::WebContext, error::Error, handler::FromRequest};

#[derive(Debug)]
pub struct Params<T>(pub T);

impl<'a, 'r, T, C, B> FromRequest<'a, WebContext<'r, C, B>> for Params<T>
where
    T: for<'de> Deserialize<'de>,
{
    type Type<'b> = Params<T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        let params = ctx.req().body().params();
        T::deserialize(Params2::new(params)).map(Params).map_err(Into::into)
    }
}

/// lazy deserialize type.
/// it lowers the deserialization to handler function where zero copy deserialize can happen.
pub struct LazyParams<'a, T> {
    params: Params2<'a>,
    _params: PhantomData<T>,
}

impl<T> LazyParams<'_, T> {
    pub fn deserialize<'de>(&'de self) -> Result<T, Error>
    where
        T: Deserialize<'de>,
    {
        T::deserialize(self.params).map_err(Into::into)
    }
}

impl<'a, 'r, C, B, T> FromRequest<'a, WebContext<'r, C, B>> for LazyParams<'a, T> {
    type Type<'b> = LazyParams<'b, T>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(LazyParams {
            params: Params2::new(ctx.req().body().params()),
            _params: PhantomData,
        })
    }
}

#[derive(Debug)]
pub struct ParamsRef<'a>(&'a router::Params);

impl Deref for ParamsRef<'_> {
    type Target = router::Params;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a, 'r, C, B> FromRequest<'a, WebContext<'r, C, B>> for ParamsRef<'a> {
    type Type<'b> = ParamsRef<'b>;
    type Error = Error;

    #[inline]
    async fn from_request(ctx: &'a WebContext<'r, C, B>) -> Result<Self, Self::Error> {
        Ok(ParamsRef(ctx.req().body().params()))
    }
}

macro_rules! unsupported_type {
    ($trait_fn:ident, $name:expr) => {
        fn $trait_fn<V>(self, _: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            Err(de::value::Error::custom(concat!("unsupported type: ", $name)))
        }
    };
}

macro_rules! parse_single_value {
    ($trait_fn:ident, $visit_fn:ident, $tp:tt) => {
        fn $trait_fn<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            if self.params.len() != 1 {
                return Err(de::value::Error::custom(format!(
                    "wrong number of parameters: {} expected 1",
                    self.params.len()
                )));
            }

            let param = self.params.iter().next().unwrap().1;
            let v = param
                .parse()
                .map_err(|_| de::value::Error::custom(format!("can not parse {param:?} to a {}", $tp)))?;
            visitor.$visit_fn(v)
        }
    };
}

#[derive(Clone, Copy)]
pub struct Params2<'de> {
    params: &'de router::Params,
}

impl<'a> Params2<'a> {
    #[inline]
    pub fn new(params: &'a router::Params) -> Self {
        Params2 { params }
    }
}

impl<'de> Deserializer<'de> for Params2<'de> {
    type Error = de::value::Error;

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.params.iter().next() {
            Some((_, v)) => visitor.visit_borrowed_str(v),
            None => Err(de::value::Error::custom("expected at least one parameters")),
        }
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(SeqAccess {
            params: self.params.iter(),
        })
    }

    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.params.len() < len {
            Err(de::value::Error::custom(
                format!("wrong number of parameters: {} expected {}", self.params.len(), len).as_str(),
            ))
        } else {
            visitor.visit_seq(SeqAccess {
                params: self.params.iter(),
            })
        }
    }

    fn deserialize_tuple_struct<V>(self, _: &'static str, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        if self.params.len() < len {
            Err(de::value::Error::custom(
                format!("wrong number of parameters: {} expected {}", self.params.len(), len).as_str(),
            ))
        } else {
            visitor.visit_seq(SeqAccess {
                params: self.params.iter(),
            })
        }
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(MapAccess {
            params: self.params.iter(),
            current: None,
        })
    }

    fn deserialize_struct<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.params.iter().next() {
            Some((_, value)) => visitor.visit_enum(ValueEnum { value }),
            None => Err(de::value::Error::custom("expected at least one parameters")),
        }
    }

    unsupported_type!(deserialize_any, "'any'");
    unsupported_type!(deserialize_bytes, "bytes");
    unsupported_type!(deserialize_option, "Option<T>");
    unsupported_type!(deserialize_identifier, "identifier");
    unsupported_type!(deserialize_ignored_any, "ignored_any");

    parse_single_value!(deserialize_bool, visit_bool, "bool");
    parse_single_value!(deserialize_i8, visit_i8, "i8");
    parse_single_value!(deserialize_i16, visit_i16, "i16");
    parse_single_value!(deserialize_i32, visit_i32, "i32");
    parse_single_value!(deserialize_i64, visit_i64, "i64");
    parse_single_value!(deserialize_u8, visit_u8, "u8");
    parse_single_value!(deserialize_u16, visit_u16, "u16");
    parse_single_value!(deserialize_u32, visit_u32, "u32");
    parse_single_value!(deserialize_u64, visit_u64, "u64");
    parse_single_value!(deserialize_f32, visit_f32, "f32");
    parse_single_value!(deserialize_f64, visit_f64, "f64");
    parse_single_value!(deserialize_string, visit_string, "String");
    parse_single_value!(deserialize_byte_buf, visit_string, "String");
    parse_single_value!(deserialize_char, visit_char, "char");
}

struct MapAccess<'de, I> {
    params: I,
    current: Option<(&'de str, &'de str)>,
}

impl<'de, I> de::MapAccess<'de> for MapAccess<'de, I>
where
    I: Iterator<Item = (&'de str, &'de str)>,
{
    type Error = de::value::Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: de::DeserializeSeed<'de>,
    {
        self.current = self.params.next();
        match self.current {
            Some((key, _)) => Ok(Some(seed.deserialize(Key { key })?)),
            None => Ok(None),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        if let Some((_, value)) = self.current.take() {
            seed.deserialize(Value { value })
        } else {
            Err(de::value::Error::custom("unexpected item"))
        }
    }
}

struct Key<'de> {
    key: &'de str,
}

impl<'de> Deserializer<'de> for Key<'de> {
    type Error = de::value::Error;

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("Unexpected"))
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_str(self.key)
    }

    forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes
            byte_buf option unit unit_struct newtype_struct seq tuple
            tuple_struct map struct enum ignored_any
    }
}

macro_rules! parse_value {
    ($trait_fn:ident, $visit_fn:ident, $tp:tt) => {
        fn $trait_fn<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let v = self
                .value
                .parse()
                .map_err(|_| de::value::Error::custom(format!("can not parse {:?} to a {}", self.value, $tp)))?;
            visitor.$visit_fn(v)
        }
    };
}

struct Value<'de> {
    value: &'de str,
}

impl<'de> Deserializer<'de> for Value<'de> {
    type Error = de::value::Error;

    parse_value!(deserialize_bool, visit_bool, "bool");
    parse_value!(deserialize_i8, visit_i8, "i8");
    parse_value!(deserialize_i16, visit_i16, "i16");
    parse_value!(deserialize_i32, visit_i32, "i16");
    parse_value!(deserialize_i64, visit_i64, "i64");
    parse_value!(deserialize_u8, visit_u8, "u8");
    parse_value!(deserialize_u16, visit_u16, "u16");
    parse_value!(deserialize_u32, visit_u32, "u32");
    parse_value!(deserialize_u64, visit_u64, "u64");
    parse_value!(deserialize_f32, visit_f32, "f32");
    parse_value!(deserialize_f64, visit_f64, "f64");
    parse_value!(deserialize_string, visit_string, "String");
    parse_value!(deserialize_byte_buf, visit_string, "String");
    parse_value!(deserialize_char, visit_char, "char");

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value)
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_bytes(self.value.as_bytes())
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V>(self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V>(self, _: &'static str, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_tuple<V>(self, _: usize, _: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("unsupported type: tuple"))
    }

    fn deserialize_tuple_struct<V>(self, _: &'static str, _: usize, _: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("unsupported type: tuple struct"))
    }

    fn deserialize_struct<V>(self, _: &'static str, _: &'static [&'static str], _: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("unsupported type: struct"))
    }

    fn deserialize_enum<V>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_enum(ValueEnum { value: self.value })
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit()
    }

    unsupported_type!(deserialize_any, "any");
    unsupported_type!(deserialize_seq, "seq");
    unsupported_type!(deserialize_map, "map");
    unsupported_type!(deserialize_identifier, "identifier");
}

struct SeqAccess<I> {
    params: I,
}

impl<'de, I> de::SeqAccess<'de> for SeqAccess<I>
where
    I: Iterator<Item = (&'de str, &'de str)>,
{
    type Error = de::value::Error;

    fn next_element_seed<U>(&mut self, seed: U) -> Result<Option<U::Value>, Self::Error>
    where
        U: de::DeserializeSeed<'de>,
    {
        match self.params.next() {
            Some((_, value)) => Ok(Some(seed.deserialize(Value { value })?)),
            None => Ok(None),
        }
    }
}

struct ValueEnum<'de> {
    value: &'de str,
}

impl<'de> de::EnumAccess<'de> for ValueEnum<'de> {
    type Error = de::value::Error;
    type Variant = UnitVariant;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
    where
        V: de::DeserializeSeed<'de>,
    {
        Ok((seed.deserialize(Key { key: self.value })?, UnitVariant))
    }
}

struct UnitVariant;

impl<'de> de::VariantAccess<'de> for UnitVariant {
    type Error = de::value::Error;

    fn unit_variant(self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn newtype_variant_seed<T>(self, _seed: T) -> Result<T::Value, Self::Error>
    where
        T: de::DeserializeSeed<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }

    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }

    fn struct_variant<V>(self, _: &'static [&'static str], _: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        Err(de::value::Error::custom("not supported"))
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;

    use serde::Deserialize;
    use xitca_http::util::service::{handler::handler_service, router::Router};
    use xitca_unsafe_collection::futures::NowOrPanic;

    use crate::{
        App,
        http::{Request, RequestExt, Uri},
        service::{Service, fn_service},
        test::collect_string_body,
    };

    use super::*;

    #[derive(Deserialize)]
    struct MyStruct {
        key: String,
        value: String,
    }

    #[derive(Debug, Deserialize)]
    struct Test1(String, u32);

    #[derive(Debug, Deserialize)]
    struct Test2 {
        key: String,
        value: u32,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum TestEnum {
        Val1,
        Val2,
    }

    async fn handler(req: Request<RequestExt<()>>) -> Result<Request<RequestExt<()>>, Infallible> {
        Ok(req)
    }

    #[test]
    fn test_request_extract() {
        let service = Router::new()
            .insert("/:key/:value/", fn_service(handler))
            .call(())
            .now_or_panic()
            .unwrap();

        let mut req = crate::http::WebRequest::default();
        *req.uri_mut() = Uri::from_static("/name/user1/");

        let res = service.call(req).now_or_panic().unwrap();

        let params = res.body().params();

        let _: () = Deserialize::deserialize(Params2::new(params)).unwrap();

        let MyStruct { key, value } = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(key, "name");
        assert_eq!(value, "user1");

        let (key, value): (String, &str) = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(key, "name");
        assert_eq!(value, "user1");

        let s: &str = de::Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(s, "name");

        let mut req = crate::http::WebRequest::default();
        *req.uri_mut() = Uri::from_static("/name/32/");

        let res = service.call(req).now_or_panic().unwrap();
        let params = res.body().params();

        let Test1(key, value) = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(key, "name");
        assert_eq!(value, 32);

        let Test2 { key, value } = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(key, "name");
        assert_eq!(value, 32);

        #[derive(Deserialize)]
        struct T(Test1);
        let T(Test1(key, value)) = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(key, "name");
        assert_eq!(value, 32);

        let s: Result<(Test2,), _> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());

        let (key, value): (String, u8) = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(key, "name");
        assert_eq!(value, 32);

        let res: Vec<String> = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(res[0], "name");
        assert_eq!(res[1], "32");

        #[derive(Debug, Deserialize)]
        struct S2(());
        let s: Result<S2, de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_ok());

        let s: Result<(), de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_ok());

        let s: Result<(String, ()), de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_ok());
    }

    #[test]
    fn test_extract_path_single() {
        let service = Router::new()
            .insert("/name/:value/", fn_service(handler))
            .call(())
            .now_or_panic()
            .unwrap();

        let mut req = crate::http::WebRequest::default();
        *req.uri_mut() = Uri::from_static("/name/47/");

        let res = service.call(req).now_or_panic().unwrap();
        let params = res.body().params();

        let i: i8 = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(i, 47);

        let i: (i8,) = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(i, (47,));

        let i: Result<(i8, i8), _> = Deserialize::deserialize(Params2::new(params));
        assert!(i.is_err());

        #[derive(Deserialize)]
        struct Test(i8);
        let i: Test = Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(i.0, 47);
    }

    #[test]
    fn test_extract_enum() {
        let service = Router::new()
            .insert("/:val/", fn_service(handler))
            .call(())
            .now_or_panic()
            .unwrap();

        let mut req = crate::http::WebRequest::default();
        *req.uri_mut() = Uri::from_static("/val1/");

        let res = service.call(req).now_or_panic().unwrap();
        let params = res.body().params();

        let i: TestEnum = de::Deserialize::deserialize(Params2::new(params)).unwrap();
        assert_eq!(i, TestEnum::Val1);

        // let service = Router::new()
        //     .insert("/:val1/:val2/", fn_service(handler))
        //     .call(())
        //     .now_or_panic()
        //     .unwrap();
        //
        // let req = http::Request::builder()
        //     .uri("/val1/")
        //     .body(())
        //     .unwrap()
        //     .map(|_| RequestExt::<()>::default());
        //
        // let res = service.call(req).now_or_panic().unwrap();
        // let params = res.body().params();
        //
        // let i: (TestEnum, TestEnum) = de::Deserialize::deserialize(Params2::new(params)).unwrap();
        // assert_eq!(i, (TestEnum::Val1, TestEnum::Val2));
    }

    #[test]
    fn test_extract_errors() {
        let service = Router::new()
            .insert("/:value/", fn_service(handler))
            .insert("/", fn_service(handler))
            .call(())
            .now_or_panic()
            .unwrap();

        let mut req = crate::http::WebRequest::default();
        *req.uri_mut() = Uri::from_static("/name/");

        let res = service.call(req).now_or_panic().unwrap();
        let params = res.body().params();

        let s: Result<Test1, de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());
        assert!(format!("{s:?}").contains("wrong number of parameters"));

        let s: Result<Test2, de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());
        assert!(format!("{s:?}").contains("can not parse"));

        let s: Result<(String, String), de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());
        assert!(format!("{s:?}").contains("wrong number of parameters"));

        let s: Result<u32, de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());
        assert!(format!("{s:?}").contains("can not parse"));

        #[derive(Debug, Deserialize)]
        struct S {
            _inner: (String,),
        }
        let s: Result<S, de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());
        assert!(format!("{s:?}").contains("missing field `_inner`"));

        let req = crate::http::WebRequest::default();

        let res = service.call(req).now_or_panic().unwrap();
        let params = res.body().params();

        let s: Result<&str, de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());
        assert!(format!("{s:?}").contains("expected at least one parameters"));

        let s: Result<TestEnum, de::value::Error> = Deserialize::deserialize(Params2::new(params));
        assert!(s.is_err());
        assert!(format!("{s:?}").contains("expected at least one parameters"));
    }

    async fn handler2(Params(MyStruct { key, value }): Params<MyStruct>) -> &'static str {
        assert_eq!(key, "qingling");
        assert_eq!(value, "dagongren");
        "996"
    }

    #[test]
    fn from_request_extract() {
        let mut req = crate::http::WebRequest::default();
        *req.uri_mut() = Uri::from_static("/qingling/dagongren/");

        let res = App::new()
            .at("/:key/:value/", handler_service(handler2))
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .unwrap();

        let s = collect_string_body(res.into_body()).now_or_panic().unwrap();

        assert_eq!(s, "996");
    }

    #[derive(Deserialize)]
    struct Meme<'a> {
        name: &'a str,
    }

    async fn handler3(lazy: LazyParams<'_, Meme<'_>>) -> &'static str {
        let Meme { name } = lazy.deserialize().unwrap();
        assert_eq!(name, "doge");
        "such dead much unoriginal"
    }

    #[test]
    fn lazy_extract() {
        let mut req = crate::http::WebRequest::default();
        *req.uri_mut() = Uri::from_static("/meme/doge");

        let res = App::new()
            .at("/meme/:name", handler_service(handler3))
            .finish()
            .call(())
            .now_or_panic()
            .unwrap()
            .call(req)
            .now_or_panic()
            .unwrap();

        let s = collect_string_body(res.into_body()).now_or_panic().unwrap();

        assert_eq!(s, "such dead much unoriginal");
    }
}
