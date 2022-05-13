use std::{
    any::{Any, TypeId},
    collections::{HashMap, VecDeque},
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, RwLock, Weak},
    task::{Context, Poll, Waker},
};

use futures::{Future, Stream};

/// Reference-counting object based on Arc<RwLock<...>> with methods for broadcasting events
#[derive(Clone)]
pub struct EArc {
    subscribers: Arc<RwLock<Subscribers>>,
}

/// Non-owning reference to EArc
#[derive(Clone, Default)]
pub struct WEArc {
    subscribers: Weak<RwLock<Subscribers>>,
}

/// Container for event. The main purpose of this container is to ensure correct order of events.
/// Asyncronous send_event function finishes only when all copies of Event are destroyed. This allows to guarantee
/// that sent event have been processed by all handlers by the moment when send_event returns.
///
/// Event object is created as output of EventStream. There is no other way to create it.
///
/// Function which handles Event object may produce new events by calling send_event. Sometimes it's necessary to
/// ensure that these derived events are handled before new instance of source event is produced. For example if mouse
/// click produces button press event, we have to be sure that button press event is handled before next mouse click arrives.
/// To do it the send_dependent_event function may be used. It stores hidden reference to source event into produced event and
/// therefore guarantees that source event stream holds until all instances of produced event are dropped.
///
pub struct Event<EVT: 'static + Send + Sync> {
    event_box: Arc<EventBox>,
    _phantom: PhantomData<EVT>,
}

/// Asychronous stream of events of specified type generated by EArc object. The stream's next() method returns Some(Event<EVT>) while
/// source object EArc is alive and None when EArc is destroyed.
pub struct EventStream<EVT: Send + Sync + 'static> {
    event_queue: Arc<RwLock<EventBoxQueue>>,
    _phantom: PhantomData<EVT>,
}

struct EventBox {
    event_id: TypeId,
    event: Box<dyn Any + Send + Sync>,
    waker: RwLock<Option<Waker>>,
    _source: Option<Arc<EventBox>>,
}

type EventBoxQueue = EventQueue<Arc<EventBox>>;

struct EventQueue<EVT: Send + Sync> {
    detached: bool,
    waker: Option<Waker>,
    events: VecDeque<EVT>,
}

struct EventSubscribers {
    event_subscribers: Vec<Weak<RwLock<EventBoxQueue>>>,
}

struct Subscribers {
    subscribers: HashMap<TypeId, EventSubscribers>,
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
    pub fn get_event_id(&self) -> TypeId {
        self.event_id
    }
    pub fn get_event<EVT: 'static + Send + Sync>(&self) -> Option<&EVT> {
        self.event.downcast_ref()
    }
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

impl Subscribers {
    fn new() -> Self {
        Self {
            subscribers: HashMap::new(),
        }
    }
    fn subscribe(&mut self, event_id: TypeId, event_queue: Weak<RwLock<EventBoxQueue>>) {
        let event_subscribers = self
            .subscribers
            .entry(event_id)
            .or_insert(EventSubscribers::new());
        event_subscribers.subscribe(event_queue)
    }
    fn send_event(&mut self, event: Arc<EventBox>) {
        if let Some(event_subscribers) = self.subscribers.get_mut(&event.get_event_id()) {
            event_subscribers.send_event(event)
        }
    }
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

impl<EVT: 'static + Send + Sync> Clone for Event<EVT> {
    fn clone(&self) -> Self {
        Self {
            event_box: self.event_box.clone(),
            _phantom: PhantomData,
        }
    }
}

struct SendEvent {
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

impl EArc {
    pub fn new() -> Self {
        let subscribers = Arc::new(RwLock::new(Subscribers::new()));
        Self { subscribers }
    }
    fn subscribe(&self, event_id: TypeId, event_queue: Weak<RwLock<EventBoxQueue>>) {
        self.subscribers
            .write()
            .unwrap()
            .subscribe(event_id, event_queue)
    }
    fn post_event_impl<EVT: Send + Sync + 'static>(
        &self,
        event: EVT,
        source: Option<Arc<EventBox>>,
    ) {
        let event_id = TypeId::of::<EVT>();
        let event = Arc::new(EventBox::new(event_id, Box::new(event), source));
        self.subscribers.write().unwrap().send_event(event);
    }
    async fn send_event_impl<EVT: Send + Sync + 'static>(
        &self,
        event: EVT,
        source: Option<Arc<EventBox>>,
    ) {
        let event_id = TypeId::of::<EVT>();
        let event = Arc::new(EventBox::new(event_id, Box::new(event), source));
        let future = SendEvent::new(Arc::downgrade(&event));
        self.subscribers.write().unwrap().send_event(event);
        future.await
    }
    pub fn post_event<EVT: Send + Sync + 'static>(&self, event: EVT) {
        self.post_event_impl(event, None)
    }
    pub async fn send_event<EVT: Send + Sync + 'static>(&self, event: EVT) {
        self.send_event_impl(event, None).await
    }
    pub fn post_derived_event<EVT: Send + Sync + 'static, EVTSRC: Send + Sync + 'static>(
        &self,
        event: EVT,
        source: Event<EVTSRC>,
    ) {
        self.post_event_impl(event, Some(source.event_box))
    }
    pub async fn send_derived_event<EVT: Send + Sync + 'static, EVTSRC: Send + Sync + 'static>(
        &self,
        event: EVT,
        source: Event<EVTSRC>,
    ) {
        self.send_event_impl(event, Some(source.event_box)).await
    }
    pub fn downgrade(&self) -> WEArc {
        WEArc {
            subscribers: Arc::downgrade(&self.subscribers),
        }
    }
}

impl<EVT: Send + Sync> Drop for EventQueue<EVT> {
    fn drop(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }
}

impl WEArc {
    pub fn upgrade(&self) -> Option<EArc> {
        if let Some(subscribers) = self.subscribers.upgrade() {
            Some(EArc {
                subscribers: subscribers.clone(),
            })
        } else {
            None
        }
    }
}

impl<EVT: Send + Sync + 'static> EventStream<EVT> {
    pub fn new(earc: &EArc) -> Self {
        let event_queue = Arc::new(RwLock::new(EventBoxQueue::new()));
        let weak_event_queue = Arc::downgrade(&mut (event_queue.clone()));
        earc.subscribe(TypeId::of::<EVT>().into(), weak_event_queue);
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
