pub mod auth;
pub mod cloud;
pub mod nodes;

pub mod error;

use core::fmt;
use core::ops::{Deref, DerefMut};
use minicbor::decode::{self, Decoder};
use minicbor::encode::{self, Encoder, Write};
use minicbor::{Decode, Encode};
use ockam_core::compat::borrow::Cow;
use ockam_core::compat::rand;
use tinyvec::ArrayVec;

/// CDDL schema or request and response headers as well as errors.
pub const SCHEMA: &str = r#"
    request  = { ?0: 7586022, 1: id, 2: path, 3: method, 4: has_body }
    response = { ?0: 9750358, 1: id, 2: re, 3: status, 4: has_body }
    error    = { ?0: 5359172, 1: path, ?2: method, ?3: message }
    id       = uint
    re       = uint
    path     = text
    method   = 0   ;; GET
             / 1   ;; POST
             / 2   ;; PUT
             / 3   ;; DELETE
             / 4   ;; PATCH
    status   = 200 ;; OK
             / 400 ;; Bad request
             / 404 ;; Not found
             / 405 ;; Method not allowed
             / 500 ;; Internal server error
             / 501 ;; Not implemented
    message  = text
    has_body = bool
"#;

/// A request header.
#[derive(Debug, Clone, Encode, Decode)]
#[rustfmt::skip]
#[cbor(map)]
pub struct Request<'a> {
    /// Nominal type tag.
    ///
    /// If the "tag" feature is enabled, the resulting CBOR will contain a
    /// unique numeric value that identifies this type to help catching type
    /// errors. Otherwise this tag will not be produced and is ignored during
    /// decoding if present.
    #[cfg(feature = "tag")]
    #[n(0)] tag: TypeTag<7586022>,
    /// The request identifier.
    #[n(1)] id: Id,
    /// The resource path.
    #[b(2)] path: Cow<'a, str>,
    /// The request method.
    ///
    /// It is wrapped in an `Option` to be forwards compatible, i.e. adding
    /// methods will not cause decoding errors and client code can decide
    /// how to handle unknown methods.
    #[n(3)] method: Option<Method>,
    /// Indicator if a request body is expected after this header.
    #[n(4)] has_body: bool
}

/// The response header.
#[derive(Debug, Clone, Encode, Decode)]
#[rustfmt::skip]
#[cbor(map)]
pub struct Response {
    /// Nominal type tag.
    ///
    /// If the "tag" feature is enabled, the resulting CBOR will contain a
    /// unique numeric value that identifies this type to help catching type
    /// errors. Otherwise this tag will not be produced and is ignored during
    /// decoding if present.
    #[cfg(feature = "tag")]
    #[n(0)] tag: TypeTag<9750358>,
    /// The response identifier.
    #[n(1)] id: Id,
    /// The identifier of the request corresponding to this response.
    #[n(2)] re: Id,
    /// A status code.
    ///
    /// It is wrapped in an `Option` to be forwards compatible, i.e. adding
    /// status codes will not cause decoding errors and client code can decide
    /// how to handle unknown codes.
    #[n(3)] status: Option<Status>,
    /// Indicator if a response body is expected after this header.
    #[n(4)] has_body: bool
}

/// A request/response identifier.
#[derive(Debug, Copy, Clone, Encode, Decode, PartialEq, Eq, PartialOrd, Ord)]
#[cbor(transparent)]
pub struct Id(#[n(0)] u32);

/// Request methods.
#[derive(Debug, Copy, Clone, Encode, Decode)]
#[non_exhaustive]
#[rustfmt::skip]
#[cbor(index_only)]
pub enum Method {
    #[n(0)] Get,
    #[n(1)] Post,
    #[n(2)] Put,
    #[n(3)] Delete,
    #[n(4)] Patch
}

/// The response status codes.
#[derive(Debug, Copy, Clone, Encode, Decode, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[rustfmt::skip]
#[cbor(index_only)]
pub enum Status {
    #[n(200)] Ok,
    #[n(400)] BadRequest,
    #[n(401)] Unauthorized,
    #[n(404)] NotFound,
    #[n(405)] MethodNotAllowed,
    #[n(500)] InternalServerError,
    #[n(501)] NotImplemented
}

/// A type tag represents a type as a unique numeric value.
///
/// This zero-sized type is meant to help catching type errors in cases where
/// CBOR items structurally match various nominal types. It will end up as an
/// unsigned integer in CBOR and decoding checks that the value is expected.
#[derive(Clone, Copy, Default)]
pub struct TypeTag<const N: usize>;

// Custom `Debug` impl to include the tag number.
impl<const N: usize> fmt::Debug for TypeTag<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TypeTag").field(&N).finish()
    }
}

impl<C, const N: usize> Encode<C> for TypeTag<N> {
    fn encode<W: Write>(
        &self,
        e: &mut Encoder<W>,
        _: &mut C,
    ) -> Result<(), encode::Error<W::Error>> {
        e.u64(N as u64)?.ok()
    }
}

impl<'b, C, const N: usize> Decode<'b, C> for TypeTag<N> {
    fn decode(d: &mut Decoder<'b>, _: &mut C) -> Result<Self, decode::Error> {
        let n = d.u64()?;
        if N as u64 == n {
            return Ok(TypeTag);
        }
        let msg = format!("type tag mismatch (expected {N}, got {n})");
        Err(decode::Error::message(msg))
    }
}

impl Id {
    pub fn fresh() -> Self {
        Id(rand::random())
    }
}

impl From<Id> for u32 {
    fn from(n: Id) -> Self {
        n.0
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}

impl<'a> Request<'a> {
    pub fn new<P: Into<Cow<'a, str>>>(method: Method, path: P, has_body: bool) -> Self {
        Request {
            #[cfg(feature = "tag")]
            tag: TypeTag,
            id: Id(rand::random()),
            method: Some(method),
            path: path.into(),
            has_body,
        }
    }

    pub fn builder<P: Into<Cow<'a, str>>>(method: Method, path: P) -> RequestBuilder<'a> {
        RequestBuilder {
            header: Request::new(method, path, false),
            body: None,
        }
    }

    pub fn get<P: Into<Cow<'a, str>>>(path: P) -> RequestBuilder<'a> {
        Request::builder(Method::Get, path)
    }

    pub fn post<P: Into<Cow<'a, str>>>(path: P) -> RequestBuilder<'a> {
        Request::builder(Method::Post, path)
    }

    pub fn put<P: Into<Cow<'a, str>>>(path: P) -> RequestBuilder<'a> {
        Request::builder(Method::Put, path)
    }

    pub fn delete<P: Into<Cow<'a, str>>>(path: P) -> RequestBuilder<'a> {
        Request::builder(Method::Delete, path)
    }

    pub fn patch<P: Into<Cow<'a, str>>>(path: P) -> RequestBuilder<'a> {
        Request::builder(Method::Patch, path)
    }

    pub fn id(&self) -> Id {
        self.id
    }

    pub fn path(&self) -> &str {
        &*self.path
    }

    pub fn path_segments<const N: usize>(&self) -> Segments<N> {
        Segments::parse(self.path())
    }

    pub fn method(&self) -> Option<Method> {
        self.method
    }

    pub fn has_body(&self) -> bool {
        self.has_body
    }
}

impl Response {
    pub fn new(re: Id, status: Status, has_body: bool) -> Self {
        Response {
            #[cfg(feature = "tag")]
            tag: TypeTag,
            id: Id(rand::random()),
            re,
            status: Some(status),
            has_body,
        }
    }

    pub fn builder(re: Id, status: Status) -> ResponseBuilder {
        ResponseBuilder {
            header: Response::new(re, status, false),
            body: None,
        }
    }

    pub fn ok(re: Id) -> ResponseBuilder {
        Response::builder(re, Status::Ok)
    }

    pub fn bad_request(re: Id) -> ResponseBuilder {
        Response::builder(re, Status::BadRequest)
    }

    pub fn not_found(re: Id) -> ResponseBuilder {
        Response::builder(re, Status::NotFound)
    }

    pub fn not_implemented(re: Id) -> ResponseBuilder {
        Response::builder(re, Status::NotImplemented)
    }

    pub fn id(&self) -> Id {
        self.id
    }

    pub fn re(&self) -> Id {
        self.re
    }

    pub fn status(&self) -> Option<Status> {
        self.status
    }

    pub fn has_body(&self) -> bool {
        self.has_body
    }
}

/// An error type used in response bodies.
#[derive(Debug, Clone, Encode, Decode)]
#[rustfmt::skip]
#[cbor(map)]
pub struct Error<'a> {
    /// Nominal type tag.
    ///
    /// If the "tag" feature is enabled, the resulting CBOR will contain a
    /// unique numeric value that identifies this type to help catching type
    /// errors. Otherwise this tag will not be produced and is ignored during
    /// decoding if present.
    #[cfg(feature = "tag")]
    #[n(0)] tag: TypeTag<5359172>,
    /// The resource path of this error.
    #[b(1)] path: Cow<'a, str>,
    /// The request method of this error.
    #[n(2)] method: Option<Method>,
    /// The actual error message.
    #[b(3)] message: Option<Cow<'a, str>>,
}

impl<'a> Error<'a> {
    pub fn new<S: Into<Cow<'a, str>>>(path: S) -> Self {
        Error {
            #[cfg(feature = "tag")]
            tag: TypeTag,
            method: None,
            path: path.into(),
            message: None,
        }
    }

    pub fn with_method(mut self, m: Method) -> Self {
        self.method = Some(m);
        self
    }

    pub fn with_message<S: Into<Cow<'a, str>>>(mut self, m: S) -> Self {
        self.message = Some(m.into());
        self
    }

    pub fn path(&self) -> &str {
        &*self.path
    }

    pub fn method(&self) -> Option<Method> {
        self.method
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

/// Path segments, i.e. '/'-separated string slices.
pub struct Segments<'a, const N: usize>(ArrayVec<[&'a str; N]>);

impl<'a, const N: usize> Segments<'a, N> {
    pub fn parse(s: &'a str) -> Self {
        if s.starts_with('/') {
            Self(s.trim_start_matches('/').splitn(N, '/').collect())
        } else {
            Self(s.splitn(N, '/').collect())
        }
    }

    pub fn as_slice(&self) -> &[&'a str] {
        &self.0[..]
    }
}

#[derive(Debug)]
pub struct RequestBuilder<'a, T = ()> {
    header: Request<'a>,
    body: Option<T>,
}

impl<'a, T> RequestBuilder<'a, T> {
    pub fn id(mut self, id: Id) -> Self {
        self.header.id = id;
        self
    }

    pub fn path<P: Into<Cow<'a, str>>>(mut self, path: P) -> Self {
        self.header.path = path.into();
        self
    }

    pub fn method(mut self, m: Method) -> Self {
        self.header.method = Some(m);
        self
    }

    pub fn header(&self) -> &Request<'a> {
        &self.header
    }

    pub fn into_parts(self) -> (Request<'a>, Option<T>) {
        (self.header, self.body)
    }
}

impl<'a> RequestBuilder<'a, ()> {
    pub fn body<T: Encode<()>>(self, b: T) -> RequestBuilder<'a, T> {
        let mut b = RequestBuilder {
            header: self.header,
            body: Some(b),
        };
        b.header.has_body = true;
        b
    }
}

impl<'a, T: Encode<()>> RequestBuilder<'a, T> {
    pub fn encode<W>(&self, buf: W) -> Result<(), encode::Error<W::Error>>
    where
        W: Write,
    {
        let mut e = Encoder::new(buf);
        e.encode(&self.header)?;
        if let Some(b) = &self.body {
            e.encode(b)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ResponseBuilder<T = ()> {
    header: Response,
    body: Option<T>,
}

impl<T> ResponseBuilder<T> {
    pub fn id(mut self, id: Id) -> Self {
        self.header.id = id;
        self
    }

    pub fn re(mut self, re: Id) -> Self {
        self.header.re = re;
        self
    }

    pub fn status(mut self, s: Status) -> Self {
        self.header.status = Some(s);
        self
    }

    pub fn header(&self) -> &Response {
        &self.header
    }

    pub fn into_parts(self) -> (Response, Option<T>) {
        (self.header, self.body)
    }
}

impl ResponseBuilder<()> {
    pub fn body<T: Encode<()>>(self, b: T) -> ResponseBuilder<T> {
        let mut b = ResponseBuilder {
            header: self.header,
            body: Some(b),
        };
        b.header.has_body = true;
        b
    }
}

impl<T: Encode<()>> ResponseBuilder<T> {
    pub fn encode<W>(&self, buf: W) -> Result<(), encode::Error<W::Error>>
    where
        W: Write,
    {
        let mut e = Encoder::new(buf);
        e.encode(&self.header)?;
        if let Some(b) = &self.body {
            e.encode(b)?;
        }
        Ok(())
    }
}

/// A newtype around `Cow<'_, str>` that borrows from input.
///
/// Contrary to `Cow<_, str>` the `Decode` impl for this type will always borrow
/// from input so using it in types like `Option`, `Vec<_>` etc will not produce
/// owned element values.
#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cbor(transparent)]
pub struct CowStr<'a>(#[b(0)] pub Cow<'a, str>);

impl CowStr<'_> {
    pub fn is_borrowed(&self) -> bool {
        matches!(self.0, Cow::Borrowed(_))
    }

    pub fn to_owned<'r>(&self) -> CowStr<'r> {
        CowStr(Cow::Owned(self.0.to_string()))
    }

    pub fn into_owned(self) -> String {
        self.0.into_owned()
    }
}

impl<'a> From<&'a str> for CowStr<'a> {
    fn from(s: &'a str) -> Self {
        CowStr(Cow::Borrowed(s))
    }
}

impl From<String> for CowStr<'_> {
    fn from(s: String) -> Self {
        CowStr(Cow::Owned(s))
    }
}

impl<'a> From<CowStr<'a>> for Cow<'a, str> {
    fn from(c: CowStr<'a>) -> Self {
        c.0
    }
}

impl<'a> Deref for CowStr<'a> {
    type Target = Cow<'a, str>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> DerefMut for CowStr<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A newtype around `Cow<'_, [u8]>` that borrows from input.
///
/// Contrary to `Cow<_, [u8]>` the `Decode` impl for this type will always borrow
/// from input so using it in types like `Option`, `Vec<_>` etc will not produce
/// owned element values.
#[derive(Debug, Clone, Encode, Decode, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cbor(transparent)]
pub struct CowBytes<'a>(#[cbor(b(0), with = "minicbor::bytes")] pub Cow<'a, [u8]>);

impl CowBytes<'_> {
    pub fn is_borrowed(&self) -> bool {
        matches!(self.0, Cow::Borrowed(_))
    }

    pub fn to_owned<'r>(&self) -> CowBytes<'r> {
        CowBytes(Cow::Owned(self.0.to_vec()))
    }

    pub fn into_owned(self) -> Vec<u8> {
        self.0.into_owned()
    }
}

impl<'a> From<&'a [u8]> for CowBytes<'a> {
    fn from(s: &'a [u8]) -> Self {
        CowBytes(Cow::Borrowed(s))
    }
}

impl From<Vec<u8>> for CowBytes<'_> {
    fn from(s: Vec<u8>) -> Self {
        CowBytes(Cow::Owned(s))
    }
}

impl<'a> From<CowBytes<'a>> for Cow<'a, [u8]> {
    fn from(c: CowBytes<'a>) -> Self {
        c.0
    }
}

impl<'a> Deref for CowBytes<'a> {
    type Target = Cow<'a, [u8]>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> DerefMut for CowBytes<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}