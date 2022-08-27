use std::{
    any::{Any, TypeId},
    marker::PhantomData,
    ops::Deref,
    sync::{Arc, RwLock},
    task::Waker,
};

/// Reference-counting container with event. Each [EventStream] instance receives clone of ```Event<T>``` referencing the same instance of ```T```.
/// When all instances of ```Event<T>``` are dropped, the [SentEvent] future returned by [send_event](EventStreams::send_event) is released.
///
/// ```Event<T>``` object can't be constructed, it can be acquired from [EventStream] only. This can't be avoided because ```Event``` contains [std::task::Waker] object.
/// This ```Waker``` allows aynchronous [EventStream::send_event] to continue when last copy of the event is dropped.
/// More precisely: [SentEvent] object returned by ```EventStream::send_event``` checks is [EventBox] instance (shared by all ```Event```s) destroyed
/// and the waker notifies ```SentEvent``` when to do the poll. Thus ```EventObject``` and so ```Event<T>``` is inseparable from it's ```EventStream```
/// and can't be put to another stream as is.
pub struct Event<EVT: 'static + Send + Sync> {
    event_box: Arc<EventBox>,
    _phantom: PhantomData<EVT>,
}

impl<EVT: 'static + Send + Sync> Event<EVT> {
    pub(crate) fn new(event_box: Arc<EventBox>) -> Self {
        Self {
            event_box,
            _phantom: PhantomData,
        }
    }
}

impl<EVT: 'static + Send + Sync> AsRef<EVT> for Event<EVT> {
    fn as_ref(&self) -> &EVT {
        &self.event_box.get_event().unwrap()
    }
}

impl<EVT: 'static + Send + Sync> Deref for Event<EVT> {
    type Target = EVT;

    fn deref(&self) -> &Self::Target {
        &self.event_box.get_event().unwrap()
    }
}

impl<EVT: 'static + Send + Sync> Into<Option<Arc<EventBox>>> for Event<EVT> {
    fn into(self) -> Option<Arc<EventBox>> {
        Some(self.event_box.clone())
    }
}

impl<EVT: 'static + Send + Sync> Clone for Event<EVT> {
    fn clone(&self) -> Self {
        Self {
            event_box: self.event_box.clone(),
            _phantom: PhantomData,
        }
    }
}
/// Clonable container with instance of event which hides type of event.
/// The [Event] object is just wrapper over ```Arc<EventBox>``` with type
/// of event specified
pub struct EventBox {
    event_id: TypeId,
    event: Box<dyn Any + Send + Sync>,
    pub(crate) waker: RwLock<Option<Waker>>,
    _source: Option<Arc<EventBox>>,
}

impl Drop for EventBox {
    fn drop(&mut self) {
        if let Some(waker) = self.waker.write().unwrap().take() {
            waker.wake()
        }
    }
}

impl EventBox {
    pub(crate) fn new(
        event_id: TypeId,
        event: Box<dyn Any + Send + Sync>,
        source: Option<Arc<EventBox>>,
    ) -> Self {
        Self {
            event_id,
            event,
            waker: RwLock::new(None),
            _source: source,
        }
    }
    /// Get [TypeId] of event object stored in box
    pub fn get_event_id(&self) -> TypeId {
        self.event_id
    }
    /// Access to event obkect stored in box. Return ```None``` if box contains event of type
    /// other than requested
    pub fn get_event<EVT: 'static + Send + Sync>(&self) -> Option<&EVT> {
        self.event.downcast_ref()
    }
}
