use std::{
    marker::PhantomData,
    pin::Pin,
    sync::{Arc, RwLock},
    task::{Context, Poll},
};

use futures::Stream;

use crate::{event::Event, event_queue::EventBoxQueue};

/// Asychronous stream of events of specified type. The stream's ```next()``` method returns ```Some(Event<EVT>)``` while
/// source object [EventStreams] is alive and ```None``` when it is destroyed.
pub struct EventStream<EVT: Send + Sync + 'static> {
    event_queue: Arc<RwLock<EventBoxQueue>>,
    _phantom: PhantomData<EVT>,
}

impl<EVT: Send + Sync + 'static> EventStream<EVT> {
    pub(crate) fn new(event_queue: Arc<RwLock<EventBoxQueue>>) -> Self {
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
