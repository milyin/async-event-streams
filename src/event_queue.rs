use std::{collections::VecDeque, sync::Arc, task::Waker};

use crate::event::EventBox;

pub(crate) struct EventBoxQueue {
    detached: bool,
    waker: Option<Waker>,
    events: VecDeque<Arc<EventBox>>,
}

impl EventBoxQueue {
    pub fn new() -> Self {
        Self {
            detached: false,
            waker: None,
            events: VecDeque::new(),
        }
    }
    pub fn is_detached(&self) -> bool {
        self.detached
    }
    pub fn detach(&mut self) {
        self.detached = true;
        self.wake();
    }
    pub fn wake(&mut self) {
        self.waker.take().map(|w| w.wake());
    }
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker)
    }
    pub fn put_event(&mut self, event: Arc<EventBox>) {
        self.events.push_back(event);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
    pub fn get_event(&mut self) -> Option<Arc<EventBox>> {
        self.events.pop_front()
    }
    pub fn clear(&mut self) {
        self.events.clear()
    }
}

impl Drop for EventBoxQueue {
    fn drop(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }
}
