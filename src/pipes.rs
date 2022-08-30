use std::{borrow::Cow, sync::Arc};

use async_std::stream::StreamExt;
use async_trait::async_trait;
use futures::task::{Spawn, SpawnError, SpawnExt};

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
pub trait EventSinkExt<EVT: Send + Sync + 'static + ToOwned> {
    type Error;
    async fn on_event(
        &self,
        event: Cow<EVT>,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error>;
}

#[async_trait]
impl<EVT: Send + Sync + ToOwned<Owned = EVT> + 'static, T: EventSinkExt<EVT> + Send + Sync>
    EventSink<EVT> for T
{
    type Error = T::Error;
    async fn on_event_owned(
        &self,
        event: EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        T::on_event(&self, Cow::Owned(event), source).await
    }
    async fn on_event_ref(
        &self,
        event: &EVT,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        T::on_event(&self, Cow::Borrowed(event), source).await
    }
}

/// Connect [EventSource] to [EventSink]: run asynchronous task which reads events from source and 
/// 
/// 
pub fn spawn_event_pipe<
    EVT: Send + Sync + Unpin + 'static,
    E,
    SPAWNER: Spawn,
    SOURCE: EventSource<EVT> + 'static,
    SINK: EventSink<EVT, Error = E> + Send + Sync + 'static,
>(
    spawner: SPAWNER,
    source: SOURCE,
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
