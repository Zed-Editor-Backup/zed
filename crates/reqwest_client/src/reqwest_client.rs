use std::{
    any::type_name,
    borrow::Cow,
    io::{self, Read},
    pin::Pin,
    task::Poll,
    thread::{self, JoinHandle},
};

use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use futures::{
    channel::{mpsc, oneshot},
    AsyncRead, StreamExt, TryStreamExt,
};
use http_client::{http, ReadTimeout};
use reqwest::{
    header::{HeaderMap, HeaderValue},
    RequestBuilder, Response,
};
use smol::future::FutureExt;

const DEFAULT_CAPACITY: usize = 4096;

pub struct ReqwestClient {
    client: reqwest::Client,
    proxy: Option<http::Uri>,
    tokio_tx: Option<
        mpsc::UnboundedSender<(
            RequestBuilder,
            oneshot::Sender<Result<Response, reqwest::Error>>,
        )>,
    >,
    _thread: Option<JoinHandle<io::Result<()>>>,
}

impl ReqwestClient {
    pub fn new() -> Self {
        reqwest::Client::new().into()
    }

    pub fn user_agent(agent: &str) -> anyhow::Result<Self> {
        let mut map = HeaderMap::new();
        map.insert(http::header::USER_AGENT, HeaderValue::from_str(agent)?);
        let client = reqwest::Client::builder().default_headers(map).build()?;
        Ok(client.into())
    }

    pub fn proxy_and_user_agent(proxy: Option<http::Uri>, agent: &str) -> anyhow::Result<Self> {
        let mut map = HeaderMap::new();
        map.insert(http::header::USER_AGENT, HeaderValue::from_str(agent)?);
        let client = reqwest::Client::builder().default_headers(map).build()?;
        let mut client: ReqwestClient = client.into();
        client.proxy = proxy;
        Ok(client)
    }
}

impl From<reqwest::Client> for ReqwestClient {
    fn from(client: reqwest::Client) -> Self {
        let has_tokio = tokio::runtime::Handle::try_current().is_ok();

        if has_tokio {
            Self {
                client,
                proxy: None,
                tokio_tx: None,
                _thread: None,
            }
        } else {
            let (sender, mut reciever) = mpsc::unbounded();
            Self {
                client,
                proxy: None,
                tokio_tx: Some(sender),
                _thread: Some(thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;

                    runtime.block_on(async {
                        while let Some((request, response_channel)) = reciever.next().await {
                            tokio::spawn(async {
                                response_channel.send(request.send().await).ok();
                            });
                        }
                    });

                    Ok(())
                })),
            }
        }
    }
}

// This struct is essentially a re-implementation of
// https://docs.rs/tokio-util/0.7.12/tokio_util/io/struct.ReaderStream.html
// except outside of Tokio's aegis
struct StreamReader {
    reader: Option<Pin<Box<dyn futures::AsyncRead + Send + Sync>>>,
    buf: BytesMut,
    capacity: usize,
}

impl StreamReader {
    fn new(reader: Pin<Box<dyn futures::AsyncRead + Send + Sync>>) -> Self {
        Self {
            reader: Some(reader),
            buf: BytesMut::new(),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl futures::Stream for StreamReader {
    type Item = std::io::Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut();

        let mut reader = match this.reader.take() {
            Some(r) => r,
            None => return Poll::Ready(None),
        };

        if this.buf.capacity() == 0 {
            let capacity = this.capacity;
            this.buf.reserve(capacity);
        }

        match poll_read_buf(&mut reader, cx, &mut this.buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(err)) => {
                self.reader = None;

                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(Ok(0)) => {
                self.reader = None;
                Poll::Ready(None)
            }
            Poll::Ready(Ok(_)) => {
                let chunk = this.buf.split();
                self.reader = Some(reader);
                Poll::Ready(Some(Ok(chunk.freeze())))
            }
        }
    }
}

/// Implementation from https://docs.rs/tokio-util/0.7.12/src/tokio_util/util/poll_buf.rs.html
/// Specialized for this use case
pub fn poll_read_buf(
    io: &mut Pin<Box<dyn futures::AsyncRead + Send + Sync>>,
    cx: &mut std::task::Context<'_>,
    buf: &mut BytesMut,
) -> Poll<std::io::Result<usize>> {
    if !buf.has_remaining_mut() {
        return Poll::Ready(Ok(0));
    }

    let n = {
        let dst = buf.chunk_mut();

        // Safety: `chunk_mut()` returns a `&mut UninitSlice`, and `UninitSlice` is a
        // transparent wrapper around `[MaybeUninit<u8>]`.
        let dst = unsafe { &mut *(dst as *mut _ as *mut [std::mem::MaybeUninit<u8>]) };
        let mut buf = tokio::io::ReadBuf::uninit(dst);
        let ptr = buf.filled().as_ptr();
        let unfilled_portion = buf.initialize_unfilled();
        // SAFETY: Pin projection
        let io_pin = unsafe { Pin::new_unchecked(io) };
        std::task::ready!(io_pin.poll_read(cx, unfilled_portion)?);

        // Ensure the pointer does not change from under us
        assert_eq!(ptr, buf.filled().as_ptr());
        buf.filled().len()
    };

    // Safety: This is guaranteed to be the number of initialized (and read)
    // bytes due to the invariants provided by `ReadBuf::filled`.
    unsafe {
        buf.advance_mut(n);
    }

    Poll::Ready(Ok(n))
}

struct SyncReader {
    cursor: Option<std::io::Cursor<Cow<'static, [u8]>>>,
}

impl SyncReader {
    fn new(cursor: std::io::Cursor<Cow<'static, [u8]>>) -> Self {
        Self {
            cursor: Some(cursor),
        }
    }
}

impl futures::stream::Stream for SyncReader {
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let Some(mut cursor) = self.cursor.take() else {
            return Poll::Ready(None);
        };

        let mut buf = Vec::new();
        match cursor.read_to_end(&mut buf) {
            Ok(_) => {
                return Poll::Ready(Some(Ok(Bytes::from(buf))));
            }
            Err(e) => return Poll::Ready(Some(Err(e))),
        }
    }
}

impl http_client::HttpClient for ReqwestClient {
    fn proxy(&self) -> Option<&http::Uri> {
        self.proxy.as_ref()
    }

    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }

    fn send(
        &self,
        req: http::Request<http_client::AsyncBody>,
    ) -> futures::future::BoxFuture<
        'static,
        Result<http_client::Response<http_client::AsyncBody>, anyhow::Error>,
    > {
        let (parts, body) = req.into_parts();

        let mut request = self.client.request(parts.method, parts.uri.to_string());

        request = request.headers(parts.headers);

        if let Some(redirect_policy) = parts.extensions.get::<http_client::RedirectPolicy>() {
            request = request.redirect_policy(match redirect_policy {
                http_client::RedirectPolicy::NoFollow => reqwest::redirect::Policy::none(),
                http_client::RedirectPolicy::FollowLimit(limit) => {
                    reqwest::redirect::Policy::limited(*limit as usize)
                }
                http_client::RedirectPolicy::FollowAll => reqwest::redirect::Policy::limited(100),
            });
        }

        if let Some(ReadTimeout(timeout)) = parts.extensions.get::<ReadTimeout>() {
            request = request.timeout(*timeout);
        }

        let request = request.body(match body.0 {
            http_client::Inner::Empty => reqwest::Body::default(),
            http_client::Inner::SyncReader(cursor) => {
                reqwest::Body::wrap_stream(SyncReader::new(cursor))
            }
            http_client::Inner::AsyncReader(stream) => {
                reqwest::Body::wrap_stream(StreamReader::new(stream))
            }
        });

        let tokio_tx = self.tokio_tx.clone();
        async move {
            let response = match tokio_tx {
                Some(tokio_tx) => {
                    let (tx, rx) = oneshot::channel();
                    tokio_tx.unbounded_send((request, tx))?;
                    rx.await?
                }
                None => request.send().await,
            }
            .map_err(|e| anyhow!(e))?;

            let status = response.status();
            let mut builder = http::Response::builder().status(status.as_u16());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            let bytes = response.bytes_stream();
            let bytes = bytes
                .map_err(|e| futures::io::Error::new(futures::io::ErrorKind::Other, e))
                .into_async_read();
            let body = http_client::AsyncBody::from_reader(bytes);
            builder.body(body).map_err(|e| anyhow!(e))
        }
        .boxed()
    }
}