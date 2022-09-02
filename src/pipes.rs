use std::{borrow::Cow, marker::PhantomData, ops::Deref, sync::Arc};

use async_std::stream::StreamExt;
use async_trait::async_trait;
use futures::{
    future::RemoteHandle,
    task::{Spawn, SpawnError, SpawnExt},
};

use crate::{EventBox, EventStream};

///
/// Standartized interface for structures providing sream of events of specified type. Typically this trait is implemented like this:
///
/// ```
/// # use async_event_streams::{EventStream, EventStreams, EventSource};
/// struct NumericSource {
///     stream: EventStreams<usize>
/// }
///
/// impl EventSource<usize> for NumericSource {
///   fn event_stream(&self) -> EventStream<usize> {
///     self.stream.create_event_stream()
///   }
/// }
/// ```
pub trait EventSource<EVT: Send + Sync + 'static> {
    fn event_stream(&self) -> EventStream<EVT>;
}

/// Standartized interface for object reacting to events of specific type. The trait have two methods: ```on_event_owned``` which accepts
/// event object and ```on_event_ref```, accepting borrowed reference to event. It's supposed both bethods should work identically.
/// But if it is necessary to retranslate the event received, it is more effective to handle owned and borrowed cases separately.
///
/// See also [EventSinkExt] trait which allows to implement only one event handler by using [Cow]
#[async_trait]
pub trait EventSink<EVT: Send + Sync + 'static> {
    type Error;
    async fn on_event_owned(
        &self,
        event: EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error>;
    async fn on_event_ref(
        &self,
        event: &EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error>;
}

/// If the event object implements [ToOwned] trait (note that all [Clone] object implements [ToOwned]), [EventSink] interface
/// can be implemented with only one event handler with [Cow] parameter, instead of separate handlers for owned and borrowed cases
#[async_trait]
pub trait EventSinkExt<EVT: Send + Sync + 'static + ToOwned<Owned = EVT>> {
    type Error;
    async fn on_event<'a>(
        &'a self,
        event: Cow<'a, EVT>,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error>;
    fn into_event_sink(self) -> IntoEventSink<EVT, Self>
    where
        Self: Sized + Send + Sync,
    {
        IntoEventSink(self, PhantomData::default())
    }
}
pub struct IntoEventSink<
    EVT: Send + Sync + ToOwned<Owned = EVT> + 'static,
    T: EventSinkExt<EVT> + Send + Sync,
>(T, PhantomData<EVT>);

#[async_trait]
impl<EVT: Send + Sync + ToOwned<Owned = EVT> + 'static, T: EventSinkExt<EVT> + Send + Sync>
    EventSink<EVT> for IntoEventSink<EVT, T>
{
    type Error = T::Error;
    async fn on_event_owned(
        &self,
        event: EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        T::on_event(&self.0, Cow::Owned(event), source).await
    }
    async fn on_event_ref(
        &self,
        event: &EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        T::on_event(&self.0, Cow::Borrowed(event), source).await
    }
}

pub struct IntoEventSinkDeref<
    EVT: Send + Sync + ToOwned<Owned = EVT> + 'static,
    T: EventSinkExt<EVT> + Send + Sync,
    Q: Deref<Target = T>,
>(Q, PhantomData<EVT>);

pub trait _IntoEventSinkDeref<
    EVT: Send + Sync + 'static + ToOwned<Owned = EVT>,
    T: EventSinkExt<EVT> + Send + Sync,
>
{
    fn into_event_sink_deref(self) -> IntoEventSinkDeref<EVT, T, Self>
    where
        Self: Sized + Send + Sync + Deref<Target = T>;
}

impl<
        EVT: Send + Sync + ToOwned<Owned = EVT> + 'static,
        T: EventSinkExt<EVT> + Send + Sync,
        Q: Deref<Target = T>,
    > _IntoEventSinkDeref<EVT, T> for Q
{
    fn into_event_sink_deref(self) -> IntoEventSinkDeref<EVT, T, Self>
    where
        Self: Sized,
    {
        IntoEventSinkDeref(self, PhantomData::default())
    }
}

#[async_trait]
impl<
        EVT: Send + Sync + ToOwned<Owned = EVT> + 'static,
        T: EventSinkExt<EVT> + Send + Sync,
        Q: Deref<Target = T> + Send + Sync,
    > EventSink<EVT> for IntoEventSinkDeref<EVT, T, Q>
{
    type Error = T::Error;
    async fn on_event_owned(
        &self,
        event: EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        T::on_event(&self.0, Cow::Owned(event), source).await
    }
    async fn on_event_ref(
        &self,
        event: &EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        T::on_event(&self.0, Cow::Borrowed(event), source).await
    }
}

/// Connect [EventSource] to [EventSink]: run asynchronous task which reads events from source and
/// calls [EventSink::on_event_ref] on sink object. Source may provide events for multiple readers, so
/// only references to events are available from it.
pub fn spawn_event_pipe<
    EVT: Send + Sync + Unpin + 'static,
    E,
    SPAWNER: Spawn,
    SOURCE: EventSource<EVT>,
    SINK: EventSink<EVT, Error = E> + Send + Sync + 'static,
>(
    spawner: &SPAWNER,
    source: &SOURCE,
    sink: SINK,
    error_handler: impl FnOnce(E) + Send + 'static,
) -> Result<(), SpawnError> {
    let mut source = source.event_stream();
    let process_events = async move {
        while let Some(event) = source.next().await {
            let eventref = event.clone();
            let eventref = &*eventref;
            sink.on_event_ref(eventref, event.into()).await?;
        }
        Result::<(), E>::Ok(())
    };
    spawner.spawn(async move {
        if let Err(e) = process_events.await {
            error_handler(e)
        }
    })
}

/// Same as [spawn_event_pipe], but also returns handle to task spawned by [futures::task::SpawnExt::spawn_with_handle]
pub fn spawn_event_pipe_with_handle<
    EVT: Send + Sync + Unpin + 'static,
    E,
    SPAWNER: Spawn,
    SOURCE: EventSource<EVT> + 'static,
    SINK: EventSink<EVT, Error = E> + Send + Sync + 'static,
>(
    spawner: &SPAWNER,
    source: &SOURCE,
    sink: SINK,
    error_handler: impl FnOnce(E) + Send + 'static,
) -> Result<RemoteHandle<()>, SpawnError> {
    let mut source = source.event_stream();
    let process_events = async move {
        while let Some(event) = source.next().await {
            let eventref = event.clone();
            let eventref = &*eventref;
            sink.on_event_ref(eventref, event.into()).await?;
        }
        Result::<(), E>::Ok(())
    };
    spawner.spawn_with_handle(async move {
        if let Err(e) = process_events.await {
            error_handler(e)
        }
    })
}
