//!
//!  This is a library for publishing/subcribing events for multiple consumers in asynchronous environment.
//!

use std::{
    any::{Any, TypeId},
    collections::{HashMap, VecDeque},
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, RwLock, Weak},
    task::{Context, Poll, Waker},
};

use futures::{Future, Stream};

/// Container for event. The main purpose of this container is to ensure correct order of events.
/// Asyncronous send_event function finishes only when all copies of Event are destroyed. This allows to guarantee
/// that sent event have been processed by all handlers by the moment when send_event returns.
///
/// ```Event``` object is created as output of [EventStream]. There is no other way to create it.
///
/// Function which handles ```Event``` object may produce new events by calling [send_event](EArc::send_event). Sometimes it's necessary to
/// ensure that these derived events are handled before new instance of source event is produced. For example if mouse
/// click produces button press event, we have to be sure that button press event is handled before next mouse click arrives.
/// To do it the second parameter ```source``` of ```send_event``` is used. Reference to source event is stored into sent event.
///
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

/// Asychronous stream of events of specified type. The stream's ```next()``` method returns ```Some(Event<EVT>)``` while
/// source object [Subscribers] is alive and ```None``` when it is destroyed.
pub struct EventStream<EVT: Send + Sync + 'static> {
    event_queue: Arc<RwLock<EventBoxQueue>>,
    _phantom: PhantomData<EVT>,
}

/// Clonable container containing instance of specific [Event] which hides type of concrete event.
///
/// The [Event] object is just typed wrapper over ```EventObject```. It goes to public interface only to allow to
/// simplify [send_event](Subcribers::send_event) method - erase type of it's ```source```parameter which actually doesn't matter
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

type EventBoxQueue = EventQueue<Arc<EventBox>>;

struct EventQueue<EVT: Send + Sync> {
    detached: bool,
    waker: Option<Waker>,
    events: VecDeque<EVT>,
}

impl<EVT: Send + Sync> EventQueue<EVT> {
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
    fn send_event(&mut self, event: EVT) {
        self.events.push_back(event);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
    fn get_event(&mut self) -> Option<EVT> {
        self.events.pop_front()
    }
}

struct EventSubscribers {
    event_subscribers: Vec<Weak<RwLock<EventBoxQueue>>>,
}

impl Drop for EventSubscribers {
    fn drop(&mut self) {
        self.event_subscribers.iter().for_each(|w| {
            w.upgrade().map(|w| w.write().ok().map(|mut w| w.detach()));
        })
    }
}

impl EventSubscribers {
    fn new() -> Self {
        Self {
            event_subscribers: Vec::new(),
        }
    }
    #[allow(dead_code)]
    fn count(&self) -> usize {
        self.event_subscribers
            .iter()
            .filter(|w| w.strong_count() > 0)
            .count()
    }
    fn subscribe(&mut self, event_queue: Weak<RwLock<EventBoxQueue>>) {
        if let Some(empty) = self
            .event_subscribers
            .iter_mut()
            .find(|w| w.strong_count() == 0)
        {
            *empty = event_queue;
        } else {
            self.event_subscribers.push(event_queue);
        };
    }
    fn send_event(&mut self, event: Arc<EventBox>) {
        self.event_subscribers
            .iter()
            .filter_map(|event_queue| event_queue.upgrade())
            .for_each(|event_queue| {
                event_queue.write().unwrap().send_event(event.clone());
            });
    }
}

/// The main object which holds separate event queue for each subscriber ([EventStream] instance)
/// and allows to send events to them
pub struct Subscribers {
    subscribers: RwLock<HashMap<TypeId, EventSubscribers>>,
}

impl Subscribers {
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(HashMap::new()),
        }
    }
    fn put_event(&self, event: Arc<EventBox>) {
        // Put event to corresponding quieue only if queue exists, i.e. there are subscribers for this typ of event.
        // Otherwise just forget this event
        if let Some(event_subscribers) = self
            .subscribers
            .write()
            .unwrap()
            .get_mut(&event.get_event_id())
        {
            event_subscribers.send_event(event)
        }
    }

    /// Put event to subscribers' queues and immediately return
    pub fn post_event<EVT: Send + Sync + 'static, EVTSRC: Into<Option<Arc<EventBox>>>>(
        &self,
        event: EVT,
        source: EVTSRC,
    ) {
        let event_id = TypeId::of::<EVT>();
        let event = Arc::new(EventBox::new(event_id, Box::new(event), source.into()));
        self.put_event(event);
    }
    /// Post event to subscribers queues and block until event is handled by all subscribers
    ///
    /// Optional parameter EVTSRC allows to link ```EVTSRC``` to currenlty sent event. It affects behavior of
    /// ```send_event``` call which earlier sent the ```EVTSRC``` event and now waiting for destroying all it's copies.
    /// See [EventOrdering](???) section
    pub fn send_event<EVT: Send + Sync + 'static, EVTSRC: Into<Option<Arc<EventBox>>>>(
        &self,
        event: EVT,
        source: EVTSRC,
    ) -> SendEvent {
        let event_id = TypeId::of::<EVT>();
        let event = Arc::new(EventBox::new(event_id, Box::new(event), source.into()));
        let future = SendEvent::new(Arc::downgrade(&event));
        self.put_event(event);
        future
    }
    // Create new subscription to event of type ```EVT```. Stream's ```next()``` method retruns events sent by
    // [send_event](Subscribers::send_event) or [post_event](Subscribers::post_event) methods. When [Subscribers] object
    // is destroyed, ```next()``` returns ```None```
    pub fn create_event_stream<EVT: Send + Sync + 'static>(&self) -> EventStream<EVT> {
        let event_queue = Arc::new(RwLock::new(EventBoxQueue::new()));
        let weak_event_queue = Arc::downgrade(&mut (event_queue.clone()));
        let event_id = TypeId::of::<EVT>().into();
        let mut subscribers = self.subscribers.write().unwrap();
        let event_subscribers = subscribers
            .entry(event_id)
            .or_insert(EventSubscribers::new());
        event_subscribers.subscribe(weak_event_queue);
        EventStream::new(event_queue)
    }
}

/// Future returned by [send_event](Subscribers::send_event) method
pub struct SendEvent {
    event: Weak<EventBox>,
}

impl SendEvent {
    fn new(event: Weak<EventBox>) -> Self {
        Self { event }
    }
    fn poll(&mut self, cx: &Context) -> Poll<()> {
        if let Some(event) = self.event.upgrade() {
            *event.waker.write().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

impl Future for SendEvent {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<<Self as Future>::Output> {
        self.get_mut().poll(cx)
    }
}

impl<EVT: Send + Sync> Drop for EventQueue<EVT> {
    fn drop(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }
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
