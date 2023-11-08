pub mod backend;
pub use backend::ClientBackend;
use backend::HyperBackend;

use cookie::{Cookie, CookieJar};
use http::HeaderValue;
use http_kit::{header, Method, Request, Response, Uri};
use hyper::http;
use once_cell::sync::Lazy;
use std::fmt::Debug;
use std::future::{Future, IntoFuture};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::RwLock;

type DefaultBackend = HyperBackend;

#[derive(Debug, Default)]
pub struct Client<B = DefaultBackend> {
    cookies: RwLock<CookieJar>,
    cookie_store: bool,
    backend: B,
}

impl<B: ClientBackend> Client<B> {
    pub fn method<U>(&self, method: Method, uri: U) -> RequestBuilder<B>
    where
        U: TryInto<Uri>,
        U::Error: Debug,
    {
        RequestBuilder::new(Request::new(method, uri.try_into().unwrap()), self)
    }

    pub fn cookie(self, cookie: Cookie<'static>) -> Self {
        self.set_cookie(cookie);
        self
    }

    pub fn enable_cookie_store(&mut self) {
        self.cookie_store = true;
    }

    pub fn disable_cookie_store(&mut self) {
        self.cookie_store = false;
    }

    fn set_cookie(&self, cookie: Cookie<'static>) {
        self.cookies.write().unwrap().add_original(cookie);
    }

    pub async fn send(&self, request: Request) -> http_kit::Result<Response> {
        RequestBuilder::new(request, self).await
    }
}

macro_rules! impl_client {
    ($(($name:ident,$method:tt)),*) => {
        impl <B:ClientBackend>Client<B>{
            $(
                pub fn $name<U>(&self, uri: U) -> RequestBuilder<B>
                where
                    U: TryInto<Uri>,
                    U::Error: Debug,
                {
                    self.method(Method::$method,uri)
                }
            )*
        }

        $(
            #[doc = concat!("Send a `",stringify!($method),"` request.")]
            pub fn $name<U>(uri: U) -> RequestBuilder<'static, DefaultBackend>
            where
                U: TryInto<Uri>,
                U::Error: Debug,
            {
                DEFAULT_CLIENT.$name(uri)
            }
        )*
    };
}

impl_client![(get, GET), (post, POST), (put, PUT), (delete, DELETE)];

pub struct RequestBuilder<'a, B> {
    request: Request,
    client: &'a Client<B>,
}

impl<'a, B: ClientBackend> RequestBuilder<'a, B> {
    fn new(request: Request, client: &'a Client<B>) -> Self {
        Self { request, client }
    }
}

impl<'a, B> Deref for RequestBuilder<'a, B> {
    type Target = Request;
    fn deref(&self) -> &Self::Target {
        &self.request
    }
}

impl<'a, B> DerefMut for RequestBuilder<'a, B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.request
    }
}

pub struct ResponseFuture<'a> {
    future: Pin<Box<dyn 'a + Future<Output = http_kit::Result<Response>>>>,
}

impl<'a> Future for ResponseFuture<'a> {
    type Output = http_kit::Result<Response>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.future.as_mut().poll(cx)
    }
}

impl<'a, B: ClientBackend> IntoFuture for RequestBuilder<'a, B> {
    type Output = http_kit::Result<Response>;

    type IntoFuture = ResponseFuture<'a>;

    fn into_future(mut self) -> Self::IntoFuture {
        ResponseFuture {
            future: Box::pin(async move {
                if self.client.cookie_store {
                    let cookies = self.client.cookies.read().unwrap();
                    let vec: Vec<String> =
                        cookies.iter().map(|v| v.encoded().to_string()).collect();
                    self.request.insert_header(
                        header::COOKIE,
                        HeaderValue::try_from(vec.join(";")).unwrap(),
                    );
                }

                let mut result = self.client.backend.call_endpoint(&mut self.request).await;
                if self.client.cookie_store {
                    result = result.map(|response| {
                        let mut cookies = self.client.cookies.write().unwrap();

                        for cookie in response.headers().get_all(header::SET_COOKIE) {
                            let cookie = String::from_utf8(cookie.as_bytes().to_vec()).unwrap();
                            cookies.add_original(Cookie::parse(cookie).unwrap());
                        }
                        response
                    });
                }
                result
            }),
        }
    }
}

impl Client<DefaultBackend> {
    pub fn new() -> Self {
        Self::default()
    }
}

static DEFAULT_CLIENT: Lazy<Client> = Lazy::new(|| Client::default());

#[cfg(test)]
mod test {
    use crate::Client;

    #[tokio::test]
    async fn example() {
        let client = Client::new();
        let mut response = client.get("http://example.com").await.unwrap();
        let string = response.into_string().await.unwrap();
        println!("{}", string);
    }
}
