//! Async client of the qccd API.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use qcc_domain::{ThreadEvent, ThreadId};
use qcc_protocol::Method;
use qcc_protocol::jsonrpc::{Incoming, Request, RpcError};
use qcc_protocol::methods::{
    THREAD_EVENT, ThreadEventNotification, ThreadEvents, ThreadEventsParams, ThreadUnsubscribe,
    ThreadUnsubscribeParams,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::{mpsc, oneshot};

pub use qcc_protocol as protocol;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot connect to qccd at {path}: {error}")]
    Connect { path: String, error: std::io::Error },
    #[error("connection to qccd lost")]
    Disconnected,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The daemon answered with an error.
    #[error(transparent)]
    Rpc(#[from] RpcError),
    #[error("unexpected message from qccd: {0}")]
    Protocol(#[from] serde_json::Error),
}

/// Calls waiting for their answer; `None` once the connection is gone, so a call
/// made after that fails at once instead of waiting for an answer that cannot come.
type Pending = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>>;
type Subscriptions = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<ThreadEvent>>>>;

/// A notification as the daemon sent it.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

/// One connection to the daemon. Requests may be issued concurrently.
pub struct Client {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    subscriptions: Subscriptions,
    next_id: AtomicU64,
}

impl Client {
    pub async fn connect(socket: impl AsRef<Path>) -> Result<Arc<Self>, ClientError> {
        Self::open(socket.as_ref(), None).await
    }

    /// Connect as a proxy: every notification is handed over untouched instead of
    /// being routed to an [`EventStream`], and requests go through
    /// [`Client::call_raw`]. The receiver ends when the connection does.
    pub async fn connect_raw(
        socket: impl AsRef<Path>,
    ) -> Result<(Arc<Self>, mpsc::UnboundedReceiver<Notification>), ClientError> {
        let (tap, notifications) = mpsc::unbounded_channel();
        Ok((Self::open(socket.as_ref(), Some(tap)).await?, notifications))
    }

    async fn open(
        socket: &Path,
        tap: Option<mpsc::UnboundedSender<Notification>>,
    ) -> Result<Arc<Self>, ClientError> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|error| ClientError::Connect { path: socket.display().to_string(), error })?;
        let (reader, writer) = stream.into_split();
        let client = Arc::new(Self {
            writer: tokio::sync::Mutex::new(writer),
            pending: Arc::new(Mutex::new(Some(HashMap::new()))),
            subscriptions: Arc::default(),
            next_id: AtomicU64::new(1),
        });

        let (pending, subscriptions) = (client.pending.clone(), client.subscriptions.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match Incoming::parse(&line) {
                    Ok(Incoming::Response(response)) => {
                        let Some(id) = response.id.as_u64() else {
                            continue;
                        };
                        let reply = pending.lock().unwrap().as_mut().and_then(|p| p.remove(&id));
                        if let Some(reply) = reply {
                            let result = match response.error {
                                Some(error) => Err(error),
                                None => Ok(response.result.unwrap_or(Value::Null)),
                            };
                            let _ = reply.send(result);
                        }
                    }
                    Ok(Incoming::Request(notification)) if tap.is_some() => {
                        let Request { method, params, .. } = notification;
                        if let Some(tap) = &tap
                            && tap.send(Notification { method, params }).is_err()
                        {
                            break;
                        }
                    }
                    Ok(Incoming::Request(notification)) if notification.method == THREAD_EVENT => {
                        let Ok(ThreadEventNotification { subscription, event }) =
                            serde_json::from_value(notification.params)
                        else {
                            continue;
                        };
                        let mut subscriptions = subscriptions.lock().unwrap();
                        if let Some(sender) = subscriptions.get(&subscription)
                            && sender.send(event).is_err()
                        {
                            subscriptions.remove(&subscription);
                        }
                    }
                    // Unknown notifications are ignored: additions must not break clients.
                    Ok(Incoming::Request(_)) | Err(_) => {}
                }
            }
            // Wake everyone waiting: dropping the senders reports the disconnect.
            pending.lock().unwrap().take();
            subscriptions.lock().unwrap().clear();
        });
        Ok(client)
    }

    /// Call a method and wait for its result.
    pub async fn call<M: Method>(&self, params: M::Params) -> Result<M::Result, ClientError> {
        let result = self.call_raw(M::NAME, serde_json::to_value(params)?).await?;
        Ok(serde_json::from_value(result)?)
    }

    /// Call a method by name, passing parameters and result through as JSON.
    pub async fn call_raw(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, receiver) = oneshot::channel();
        match self.pending.lock().unwrap().as_mut() {
            Some(pending) => pending.insert(id, reply),
            None => return Err(ClientError::Disconnected),
        };

        let request = Request::new(id, method, params);
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        if let Err(error) = self.writer.lock().await.write_all(line.as_bytes()).await {
            if let Some(pending) = self.pending.lock().unwrap().as_mut() {
                pending.remove(&id);
            }
            return Err(error.into());
        }

        Ok(receiver.await.map_err(|_| ClientError::Disconnected)??)
    }

    /// Read a thread's events after `after`. With `follow`, the stream continues with
    /// events as they are appended; without, it ends after the backlog.
    pub async fn events(
        self: &Arc<Self>,
        thread_id: ThreadId,
        after: u64,
        follow: bool,
    ) -> Result<EventStream, ClientError> {
        // The subscription is registered before the request so that no live event
        // can arrive ahead of its receiver.
        let live = follow.then(|| {
            let subscription = self.next_id.fetch_add(1, Ordering::Relaxed);
            let (sender, receiver) = mpsc::unbounded_channel();
            self.subscriptions.lock().unwrap().insert(subscription, sender);
            (subscription, receiver)
        });
        let subscription = live.as_ref().map(|(id, _)| *id);
        let result =
            self.call::<ThreadEvents>(ThreadEventsParams { thread_id, after, subscription }).await;
        match result {
            Ok(result) => Ok(EventStream {
                client: self.clone(),
                backlog: result.events.into(),
                live,
                last_seq: after,
            }),
            Err(error) => {
                if let Some(subscription) = subscription {
                    self.subscriptions.lock().unwrap().remove(&subscription);
                }
                Err(error)
            }
        }
    }
}

/// Events of one thread, in order and without duplicates.
pub struct EventStream {
    client: Arc<Client>,
    backlog: VecDeque<ThreadEvent>,
    live: Option<(u64, mpsc::UnboundedReceiver<ThreadEvent>)>,
    last_seq: u64,
}

impl EventStream {
    /// The next event; `None` when the backlog is exhausted and the stream does not
    /// follow, or when the connection is gone.
    pub async fn next(&mut self) -> Option<ThreadEvent> {
        loop {
            let event = match self.backlog.pop_front() {
                Some(event) => event,
                None => self.live.as_mut()?.1.recv().await?,
            };
            if event.seq > self.last_seq {
                self.last_seq = event.seq;
                return Some(event);
            }
        }
    }

    /// Stop following, and wait until the daemon has stopped sending. Dropping the
    /// stream stops following too, without waiting.
    pub async fn close(mut self) {
        if let Some((subscription, _)) = self.live.take() {
            self.client.subscriptions.lock().unwrap().remove(&subscription);
            let _ = self
                .client
                .call::<ThreadUnsubscribe>(ThreadUnsubscribeParams { subscription })
                .await;
        }
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        let Some((subscription, _)) = self.live.take() else { return };
        self.client.subscriptions.lock().unwrap().remove(&subscription);
        // Otherwise the daemon keeps sending this thread's events down the connection
        // for as long as it stays open. Outside a runtime there is nobody to ask; the
        // events are then dropped here, unread.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let client = self.client.clone();
            runtime.spawn(async move {
                let params = ThreadUnsubscribeParams { subscription };
                let _ = client.call::<ThreadUnsubscribe>(params).await;
            });
        }
    }
}
