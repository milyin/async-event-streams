use async_std::stream::StreamExt;
use async_trait::async_trait;
use futures::{
    future::RemoteHandle,
    task::{Spawn, SpawnError, SpawnExt},
};
use std::sync::Arc;

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
/// event object and ```on_event_ref```, accepting borrowed reference to event. It's supposed that both bethods should work identically.
/// But sometimes if it is necessary to retranslate the event received. So it is effective to handle owned event case separately from borrowed.
///
/// See also [crate::EventSinkExt] trait which allows to implement only one event handler by using [std::borrow::Cow]
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

#[async_trait]
impl<EVT: Send + Sync + 'static, T: EventSink<EVT> + Send + Sync> EventSink<EVT> for Arc<T> {
    type Error = T::Error;
    async fn on_event_owned(
        &self,
        event: EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        (**self).on_event_owned(event, source).await
    }
    async fn on_event_ref(
        &self,
        event: &EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        (**self).on_event_ref(event, source).await
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
