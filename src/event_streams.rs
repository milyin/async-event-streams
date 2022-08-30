use std::{
    any::TypeId,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, RwLock, Weak},
    task::{Context, Poll},
};

use futures::Future;

use crate::{event::EventBox, event_queue::EventBoxQueue, event_stream::EventStream};

/// Main object which allows to send events and subscribe to them
pub struct EventStreams<EVT: Send + Sync + 'static> {
    queues: RwLock<EventBoxQueues>,
    _phantom: PhantomData<EVT>,
}

impl<EVT: Send + Sync + 'static> EventStreams<EVT> {
    pub fn new() -> Self {
        let queues = RwLock::new(EventBoxQueues::new());
        Self {
            queues,
            _phantom: PhantomData,
        }
    }
    /// Return number of subscribers
    pub fn count(&self) -> usize {
        self.queues.read().unwrap().count()
    }
    /// Put event to streams and immediately return
    ///
    /// Parameters:
    ///
    /// - ```event``` - event itself
    /// - ```source``` - optional parameter with event which was the cause of ```event```. See [Dependent Events](#dependent-events) section for details
    ///
    pub fn post_event<EVTSRC: Into<Option<Arc<EventBox>>>>(&self, event: EVT, source: EVTSRC) {
        let mut queues = self.queues.write().unwrap();
        let event_id = TypeId::of::<EVT>();
        let event = Arc::new(EventBox::new(event_id, Box::new(event), source.into()));
        queues.put_event(event);
    }
    /// Put event to streams and return future whcih blocks until event is handled by all subscribers
    ///
    /// Parameters:
    ///
    /// - ```event``` - event itself
    /// - ```source``` - optional parameter with event which was the cause of ```event```. See [Dependent Events](#dependent-events) section for details
    ///
    pub fn send_event<EVTSRC: Into<Option<Arc<EventBox>>>>(
        &self,
        event: EVT,
        source: EVTSRC,
    ) -> SentEvent {
        let mut queues = self.queues.write().unwrap();
        let event_id = TypeId::of::<EVT>();
        let event = Arc::new(EventBox::new(event_id, Box::new(event), source.into()));
        let future = SentEvent::new(Arc::downgrade(&event));
        queues.put_event(event);
        future
    }
    // Create new subscription to event - [EventStream] object. Stream's ```next()``` method returns events sent by
    // [send_event](EventStreams::send_event) or [post_event](EventStreams::post_event) methods. When [EventStreams] object
    // is destroyed, ```next()``` returns ```None```
    pub fn create_event_stream(&self) -> EventStream<EVT> {
        let mut queues = self.queues.write().unwrap();
        let event_queue = Arc::new(RwLock::new(EventBoxQueue::new()));
        let weak_event_queue = Arc::downgrade(&mut (event_queue.clone()));
        queues.add_queue(weak_event_queue);
        EventStream::new(event_queue)
    }

    /// Remove all pending events for all subscribers
    pub fn clear(&self) {
        self.queues.write().unwrap().clear()
    }
}

/// Future returned by [send_event](EventStreams::send_event). ```await``` on it blocks until all instances of [crate::Event] sent to [EventStream]
/// by this ```send_event``` are dropped
pub struct SentEvent(Weak<EventBox>);

impl SentEvent {
    fn new(event: Weak<EventBox>) -> Self {
        Self(event)
    }
    fn poll(&mut self, cx: &Context) -> Poll<()> {
        if let Some(event) = self.0.upgrade() {
            *event.waker.write().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

impl Future for SentEvent {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<<Self as Future>::Output> {
        self.get_mut().poll(cx)
    }
}

struct EventBoxQueues(Vec<Weak<RwLock<EventBoxQueue>>>);

impl Drop for EventBoxQueues {
    fn drop(&mut self) {
        self.0.iter().for_each(|w| {
            w.upgrade().map(|w| w.write().ok().map(|mut w| w.detach()));
        })
    }
}

impl EventBoxQueues {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn count(&self) -> usize {
        self.0.iter().filter(|w| w.strong_count() > 0).count()
    }
    fn add_queue(&mut self, event_queue: Weak<RwLock<EventBoxQueue>>) {
        if let Some(empty) = self.0.iter_mut().find(|w| w.strong_count() == 0) {
            *empty = event_queue;
        } else {
            self.0.push(event_queue);
        };
    }
    fn put_event(&mut self, event: Arc<EventBox>) {
        self.0
            .iter()
            .filter_map(|event_queue| event_queue.upgrade())
            .for_each(|event_queue| {
                event_queue.write().unwrap().put_event(event.clone());
            });
    }
    fn clear(&mut self) {
        self.0
            .iter()
            .filter_map(|event_queue| event_queue.upgrade())
            .for_each(|event_queue| event_queue.write().unwrap().clear())
    }
}
