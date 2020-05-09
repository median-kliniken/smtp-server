use std::{marker::PhantomData, pin::Pin, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::{io, prelude::*};

use smtp_message::{Email, ReplyCode};

// Use cases to take into account:
//  * By mistake, multiple instances have been started with the same queue
//    directory
//  * The user wants to modify by hand data in the queue for some reason, it's
//    better not to have to shut down the server in order to do that (esp. as
//    they may forget to do it). But it's OK to require them to notify the
//    server after having done that
// Idea:
//  * Before sending mails, move them to an in-progress directory so that
//    multiple simultaneously-running instances don't send the same mail at the
//    same time.
//  * If there is a crash, a mail may be stuck in this in-progress directory.
//    So, at startup:
//     * Also scan the in-progress directory.
//     * If there is a mail there, it *could* be in the process of being sent,
//       so wait long enough (1 hour?) to be sure all timeouts are passed, and
//       check if it is still there.
//     * If it is still there, then it means that it was left here after a crash
//       while sending it, as the name in the in-progress directory is randomly
//       picked (so even if it was actually in-progress and had been
//       re-scheduled and put back in the in-progress directory, it would have a
//       new name).

pub struct MailMetadata<U> {
    pub from: Option<Email>,
    pub to: Vec<Email>,
    pub metadata: U,
}

#[derive(Clone)]
pub struct QueueId(pub Arc<String>);

#[async_trait]
pub trait Config<U>: 'static + Send + Sync {
    fn next_interval(&self, i: Duration) -> Duration;

    async fn bounce(&self, id: QueueId, meta: MailMetadata<U>, code: ReplyCode, err: io::Error);

    async fn log_permanent_error(&self, id: QueueId, c: ReplyCode, err: &io::Error);
    async fn log_transient_error(&self, id: QueueId, c: ReplyCode, err: io::Error);
    async fn log_io_error(&self, id: QueueId, err: io::Error);
    async fn log_inflight_mail_vanished(&self, id: QueueId);
    async fn log_too_big_duration(&self, id: QueueId, interval: Duration);

    // The important thing is it must be longer than the time between
    // switching a mail to inflight and either completing it or
    // returning it to the queue
    fn found_inflight_check_delay(&self) -> Duration {
        Duration::from_secs(3600)
    }

    fn io_error_next_retry_delay(&self, d: Duration) -> Duration {
        if d < Duration::from_secs(30) {
            Duration::from_secs(60)
        } else {
            d.mul_f64(2.0)
        }
    }
}

#[async_trait]
pub trait Storage<U>: 'static + Send + Sync {
    type QueuedMail: QueuedMail;
    type InflightMail: InflightMail;
    type Enqueuer: StorageEnqueuer<Self::QueuedMail>;
    type Reader: Send + AsyncRead;

    async fn list_queue(&self) -> Pin<Box<dyn Send + Stream<Item = Self::QueuedMail>>>;
    async fn find_inflight(&self) -> Pin<Box<dyn Send + Stream<Item = Self::InflightMail>>>;

    async fn read_inflight(
        &self,
        mail: Self::InflightMail,
    ) -> Result<(Self::InflightMail, MailMetadata<U>, Self::Reader), (Self::InflightMail, io::Error)>;

    async fn enqueue(&self, meta: MailMetadata<U>) -> Result<Self::Enqueuer, io::Error>;

    async fn send_start(
        &self,
        mail: Self::QueuedMail,
    ) -> Result<Self::InflightMail, (Self::QueuedMail, io::Error)>;

    async fn send_done(
        &self,
        mail: Self::InflightMail,
    ) -> Result<(), (Self::InflightMail, io::Error)>;

    async fn send_cancel(
        &self,
        mail: Self::InflightMail,
    ) -> Result<Option<Self::QueuedMail>, (Self::InflightMail, io::Error)>;
}

#[async_trait]
pub trait QueuedMail: Send + Sync {
    fn id(&self) -> QueueId;

    async fn schedule(&self, at: DateTime<Utc>) -> Result<(), io::Error>;

    fn scheduled_at(&self) -> DateTime<Utc>;
    fn last_interval(&self) -> Duration;
}

pub trait InflightMail: Send + Sync {
    fn id(&self) -> QueueId;

    fn was_scheduled_at(&self) -> DateTime<Utc>;
    fn last_interval(&self) -> Duration;
}

#[async_trait]
pub trait StorageEnqueuer<QueuedMail>: Send + AsyncWrite {
    async fn commit(self) -> Result<QueuedMail, io::Error>;
}

pub enum TransportFailure {
    Local(io::Error),
    RemoteTransient(ReplyCode, io::Error),
    RemotePermanent(ReplyCode, io::Error),
}

#[async_trait]
pub trait Transport<U>: 'static + Send + Sync {
    async fn send<Reader>(
        &self,
        meta: &MailMetadata<U>,
        mail: Reader,
    ) -> Result<(), TransportFailure>
    where
        Reader: AsyncRead;
}

const INTERVAL_ON_TOO_BIG_DURATION_HR: i64 = 4;

struct QueueImpl<C, S, T> {
    config: C,
    storage: S,
    transport: T,
}

pub struct Queue<U, C, S, T> {
    q: Arc<QueueImpl<C, S, T>>,
    phantom: PhantomData<U>,
}

macro_rules! io_retry_loop {
    ($this:ident, $init:expr, | $mail:ident | $e:expr) => {{
        let mut delay = Duration::from_secs(0);
        let mut $mail = $init;
        loop {
            match $e {
                Ok(v) => {
                    break v;
                }
                Err((mail, e)) => {
                    $this.q.config.log_io_error(mail.id(), e).await;
                    $mail = mail;
                }
            }
            smol::Timer::after(delay).await;
            delay = $this.q.config.io_error_next_retry_delay(delay);
        }
    }};
}

macro_rules! io_retry_loop_raw {
    ($this:ident, $id:expr, $e:expr) => {{
        let mut delay = Duration::from_secs(0);
        loop {
            match $e {
                Ok(v) => {
                    break v;
                }
                Err(e) => {
                    $this.q.config.log_io_error($id, e).await;
                }
            }
            smol::Timer::after(delay).await;
            delay = $this.q.config.io_error_next_retry_delay(delay);
        }
    }};
}

impl<U, C, S, T> Queue<U, C, S, T>
where
    U: 'static + Send + Sync,
    C: Config<U>,
    S: Storage<U>,
    T: Transport<U>,
{
    pub async fn new(config: C, storage: S, transport: T) -> Queue<U, C, S, T> {
        let this = Queue {
            q: Arc::new(QueueImpl {
                config,
                storage,
                transport,
            }),
            phantom: PhantomData,
        };

        this.scan_inflight().await;

        {
            let this = this.clone();
            smol::Task::spawn(async move { this.scan_queue().await }).detach();
        }

        this
    }

    pub async fn enqueue(&self, meta: MailMetadata<U>) -> Result<Enqueuer<U, C, S, T>, io::Error> {
        Ok(Enqueuer {
            queue: self.clone(),
            enqueuer: Some(self.q.storage.enqueue(meta).await?),
        })
    }

    async fn scan_inflight(&self) {
        let mut found_inflight_stream = self.q.storage.find_inflight().await;
        while let Some(inflight) = found_inflight_stream.next().await {
            let this = self.clone();
            smol::Task::spawn(async move {
                smol::Timer::after(this.q.config.found_inflight_check_delay()).await;
                let queued =
                    io_retry_loop!(this, inflight, |i| this.q.storage.send_cancel(i).await);
                match queued {
                    // Mail is still waiting, probably was inflight
                    // during a crash
                    Some(queued) => this.send(queued).await,

                    // Mail is no longer waiting, probably was
                    // inflight because another process was currently
                    // sending it
                    None => (),
                }
            })
            .detach();
        }
    }

    async fn scan_queue(&self) {
        let mut queued_stream = self.q.storage.list_queue().await;
        while let Some(queued) = queued_stream.next().await {
            let this = self.clone();
            smol::Task::spawn(async move {
                this.send(queued).await;
            })
            .detach();
        }
    }

    async fn send(&self, mail: S::QueuedMail) {
        let mut mail = mail;
        loop {
            let interval = (mail.scheduled_at() - Utc::now())
                .to_std()
                .unwrap_or(Duration::from_secs(0));
            smol::Timer::after(interval).await;
            match self.try_send(mail).await {
                Ok(()) => break,
                Err(m) => mail = m,
            }
            let interval = self.q.config.next_interval(mail.last_interval());
            let interval = match chrono::Duration::from_std(interval) {
                Ok(i) => i,
                Err(_) => {
                    self.q
                        .config
                        .log_too_big_duration(mail.id(), interval)
                        .await;
                    chrono::Duration::hours(INTERVAL_ON_TOO_BIG_DURATION_HR)
                }
            };
            io_retry_loop_raw!(self, mail.id(), mail.schedule(Utc::now() + interval).await);
        }
    }

    async fn try_send(&self, mail: S::QueuedMail) -> Result<(), S::QueuedMail> {
        let inflight = io_retry_loop!(self, mail, |m| self.q.storage.send_start(m).await);

        let (inflight, meta, reader) =
            io_retry_loop!(self, inflight, |i| self.q.storage.read_inflight(i).await);

        match self.q.transport.send(&meta, reader).await {
            Ok(()) => {
                io_retry_loop!(self, inflight, |i| self.q.storage.send_done(i).await);
                return Ok(());
            }
            Err(TransportFailure::RemotePermanent(c, e)) => {
                self.q
                    .config
                    .log_permanent_error(inflight.id(), c, &e)
                    .await;
                self.q.config.bounce(inflight.id(), meta, c, e).await;
                return Ok(());
            }
            Err(TransportFailure::Local(e)) => {
                self.q.config.log_io_error(inflight.id(), e).await;
            }
            Err(TransportFailure::RemoteTransient(c, e)) => {
                self.q.config.log_transient_error(inflight.id(), c, e).await;
            }
        }
        // The above match falls through only in cases where we ought to retry
        let id = inflight.id();
        let queued = io_retry_loop!(self, inflight, |i| self.q.storage.send_cancel(i).await);
        match queued {
            Some(queued) => Err(queued),
            None => {
                self.q.config.log_inflight_mail_vanished(id).await;
                Ok(())
            }
        }
    }
}

impl<U, C, S, T> Clone for Queue<U, C, S, T> {
    fn clone(&self) -> Self {
        Self {
            q: self.q.clone(),
            phantom: self.phantom,
        }
    }
}

pub struct Enqueuer<U, C, S, T>
where
    S: Storage<U>,
{
    queue: Queue<U, C, S, T>,
    enqueuer: Option<S::Enqueuer>,
}

impl<U, C, S, T> Enqueuer<U, C, S, T>
where
    U: 'static + Send + Sync,
    C: Config<U>,
    S: Storage<U>,
    T: Transport<U>,
{
    pub async fn commit(self) -> Result<(), io::Error> {
        let mut this = self;
        let mail = this.enqueuer.take().unwrap().commit().await?;
        smol::Task::spawn(async move { this.queue.send(mail).await }).detach();
        Ok(())
    }
}

// TODO: (B) Add tests
// TODO: (B) Add fuzzing target
