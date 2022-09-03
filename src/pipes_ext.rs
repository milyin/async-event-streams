use std::{borrow::Cow, marker::PhantomData, ops::Deref, sync::Arc};

use async_trait::async_trait;

use crate::{EventBox, EventSink};

/// If the event object implements [ToOwned] trait (note that all [Clone] object implements it), [EventSink] implementation
/// can be simplifyed by implementing helper [EventSinkExt] trait with only one event handler with [std::borrow::Cow] parameter,
/// instead of separate handlers for owned and borrowed cases
///
/// To avoid conflicts with blanket impls the [EventSink] trait is not implemented automatically for structures implementing [EventSinkExt].
/// Instead there is the method [EventSinkExt::into_event_sink] which returns wrapper [IntoEventSink], implementing the [EventSink] trait.
///
/// For boxed structures imlmementing [EventSinkExt] the another wrapper [IntoEventSinkDeref] exits. The trait [ToIntoEventSinkDeref] provides method
/// [ToIntoEventSinkDeref::into_event_sink] and this trait is automatically imlemented for any type which [Deref]s to [EventSinkExt] type.
///
/// Usage example:
/// ```
/// use std::sync::Arc;
/// use async_trait::async_trait;
/// use std::borrow::Cow;
/// use async_event_streams::{
///    EventBox, EventSink, EventSinkExt, EventSource, ToIntoEventSinkDeref,
/// };
///
/// struct Sink;
///
/// #[derive(Clone)]
/// struct Event;
///
/// #[async_trait]
/// impl EventSinkExt<Event> for Sink {
///     type Error = ();
///     async fn on_event<'a>(
///         &'a self,
///         event: Cow<'a, Event>,
///         source: Option<Arc<EventBox>>,
///     ) -> Result<(), Self::Error> {
///         todo!()
///     }
///}
///
/// let sink = Sink;
/// let sink_as_event_sink = sink.into_event_sink();
///
/// let arc_sink = Arc::new(Sink);
/// let arc_sink_as_event_sink = arc_sink.clone().into_event_sink();
///
/// // 'on_event_owned' method is from EventSink trait
/// sink_as_event_sink.on_event_owned(Event, None);
/// arc_sink_as_event_sink.on_event_owned(Event, None);
///
/// ```
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

pub trait ToIntoEventSinkDeref<
    EVT: Send + Sync + 'static + ToOwned<Owned = EVT>,
    T: EventSinkExt<EVT> + Send + Sync,
>
{
    fn into_event_sink(self) -> IntoEventSinkDeref<EVT, T, Self>
    where
        Self: Sized + Send + Sync + Deref<Target = T>;
}

impl<
        EVT: Send + Sync + ToOwned<Owned = EVT> + 'static,
        T: EventSinkExt<EVT> + Send + Sync,
        Q: Deref<Target = T>,
    > ToIntoEventSinkDeref<EVT, T> for Q
{
    fn into_event_sink(self) -> IntoEventSinkDeref<EVT, T, Self>
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
