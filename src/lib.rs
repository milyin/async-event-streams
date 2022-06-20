//!
//! Library for publishing events for multiple consumers using asynchromous streams
//!
//! # Usage sample
//!
//! ```
//! use futures::{executor::LocalPool, task::LocalSpawnExt, StreamExt};
//! use async_events::EventStreams;
//!
//! let mut pool = LocalPool::new();
//!
//! let streams = EventStreams::new();
//! let mut stream = streams.create_event_stream();
//!
//! let sender_task = async move {
//!     assert!(streams.count() == 1);
//!     streams.send_event(42, None).await;
//!     streams.send_event(451, None).await;
//!     streams.send_event(1984, None).await;
//! };
//!
//! let receiver_task = async move {
//!     let mut values = Vec::new();
//!     while let Some(event) = stream.next().await {
//!         values.push(*event.as_ref());
//!     }
//!     // next() returns none when 'streams' is dropped
//!     assert!(values == vec![42, 451, 1984]);
//! };
//!
//! pool.spawner().spawn_local(sender_task);
//! pool.spawner().spawn_local(receiver_task);
//! pool.run();
//! ```
//!
//! # Event processing order
//!
//! When event is sent it becomes immediately available for all [EventStream] objects.
//! Events covmes from stream exactly in order as they being sent. But between streams this order is not guaranteed.
//! Though it may be necessary to ensure that events are handled in order globally. I.e. for async tasks A and B the events E1,E2,E3 should be processed
//! in order A(E1), B(E1), B(E2), A(E2), B(E3), A(E3), but not in order A(E1), A(E2), A(E3), B(E1), B(E2), B(E3).
//!
//! To achieve this the [send_event](EventStreams::send_event) function returns future [SentEvent]. Each [EventStream] instance receives clone
//! of [Event<T>](Event) object. When all these clones are dropped the ```SentEvent``` future is released. This guarantees that E2 is sent only
//! when E1 has been processed by all subscribers.
//!
//! If such blocking until is not necessary, the [post_event](EventStreams::post_event) can be used instead.
//!
//! # Dependent Events
//!
//! Receiving events may cause firing new events. For example button's mouse click handler is sending button press events.
//! It may be important to guarantee that button click events are not handled in order different than mouse clicks order.
//!
//! For example consider two buttons A and B. Click event C1 causes button A send press
//! event P1, click C2 causes button B send press event P2. It's guaranteed that P2 is *sent* after P1 (because P1 is reaction to C1,
//! P2 is reaction to C2, and both C1 and C2 comes from same ```send_event```).  But there is still no guarantee that P2 is *processed* after P1,
//! because P1 and P2 arrives from different sources.
//!
//! This may be inappropriate. For example: user clicks "Apply" button and then "Close" button in the dialog. But press event from "Close"
//! button comes earlier, than from "Apply" "Close" handler destroys the dialog. "Apply" is not processed, user's data is lost.
//!
//! To avoid this the concept of "source" event is added. [send_event](EventStreams::send_event) and [post_event](EventStreams::post_event) have
//! the additional optional parameter - event which caused the sent one. Reference to this 'source' event is saved inside Event wrapper of new event
//! and therefore source ```send_event``` is blocked until all derived events are dropped. So click to "Close" in example above is not sent until
//! "Apply" handler finishes.
//!

use std::{
    any::{Any, TypeId},
    collections::VecDeque,
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, RwLock, Weak},
    task::{Context, Poll, Waker},
};

use futures::{Future, Stream};

/// Reference-counting container with event. Each [EventStream] instance receives clone of ```Event<T>``` referencing the same instance of ```T```.
/// When all instances of ```Event<T>``` are dropped, the [SentEvent] future returned by [send_event](EventStreams::send_event) is released.
///
/// ```Event<T>``` object is acquired from [EventStream] only.
pub struct Event<EVT: 'static + Send + Sync> {
    event_box: Arc<EventBox>,
    _phantom: PhantomData<EVT>,
}

impl<EVT: 'static + Send + Sync> Event<EVT> {
    fn new(event_box: Arc<EventBox>) -> Self {
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
/// The [Event] object is just typed wrapper over ```EventObject```.
pub struct EventBox {
    event_id: TypeId,
    event: Box<dyn Any + Send + Sync>,
    waker: RwLock<Option<Waker>>,
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
    fn new(
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

struct EventBoxQueue {
    detached: bool,
    waker: Option<Waker>,
    events: VecDeque<Arc<EventBox>>,
}

impl EventBoxQueue {
    fn new() -> Self {
        Self {
            detached: false,
            waker: None,
            events: VecDeque::new(),
        }
    }
    fn is_detached(&self) -> bool {
        self.detached
    }
    fn detach(&mut self) {
        self.detached = true;
        self.wake();
    }
    fn wake(&mut self) {
        self.waker.take().map(|w| w.wake());
    }
    fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker)
    }
    fn put_event(&mut self, event: Arc<EventBox>) {
        self.events.push_back(event);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
    fn get_event(&mut self) -> Option<Arc<EventBox>> {
        self.events.pop_front()
    }
}

impl Drop for EventBoxQueue {
    fn drop(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
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
}

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
}

/// Future returned by [send_event](EventStreams::send_event) method. ```await``` on it locks until all instances of sent event are
/// dropped
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

/// Asychronous stream of events of specified type. The stream's ```next()``` method returns ```Some(Event<EVT>)``` while
/// source object [EventStreams] is alive and ```None``` when it is destroyed.
pub struct EventStream<EVT: Send + Sync + 'static> {
    event_queue: Arc<RwLock<EventBoxQueue>>,
    _phantom: PhantomData<EVT>,
}

impl<EVT: Send + Sync + 'static> EventStream<EVT> {
    fn new(event_queue: Arc<RwLock<EventBoxQueue>>) -> Self {
        Self {
            event_queue,
            _phantom: PhantomData,
        }
    }
    fn poll_next(self: &mut Self, cx: &mut Context<'_>) -> Poll<Option<Event<EVT>>> {
        let mut event_queue = self.event_queue.write().unwrap();
        if event_queue.is_detached() {
            Poll::Ready(event_queue.get_event().map(|v| Event::new(v)))
        } else {
            if let Some(evt) = event_queue.get_event() {
                Poll::Ready(Some(Event::new(evt)))
            } else {
                event_queue.set_waker(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl<EVT: Send + Sync + Unpin + 'static> Stream for EventStream<EVT> {
    type Item = Event<EVT>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_next(cx)
    }
}
