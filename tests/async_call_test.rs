use async_object::CArc;
use futures::{executor::LocalPool, task::LocalSpawnExt};
use std::{cell::RefCell, rc::Rc};

struct CounterImpl {
    internal_value: usize,
}

impl CounterImpl {
    fn new() -> Self {
        Self { internal_value: 0 }
    }
    fn inc(&mut self) {
        self.internal_value += 1;
    }
    fn internal_value(&self) -> usize {
        self.internal_value
    }
}

struct Counter(CArc<CounterImpl>);

impl Counter {
    fn new() -> Self {
        Counter(CArc::new(CounterImpl::new()))
    }
    async fn inc(&self) {
        self.0
            .async_call_mut(|counter: &mut CounterImpl| counter.inc())
            .await
    }
    async fn internal_value(&self) -> usize {
        self.0
            .async_call(|counter: &CounterImpl| counter.internal_value())
            .await
    }
}

#[test]
fn test_handle_call() {
    let mut pool = LocalPool::new();
    let test_value = Rc::new(RefCell::new(None));
    let test_value_r = test_value.clone();

    let counter = Counter::new();

    let future = async move {
        let v = counter.internal_value().await;
        *(test_value.borrow_mut()) = Some(v);
    };
    pool.spawner().spawn_local(future).unwrap();
    pool.run_until_stalled();
    assert!(test_value_r.borrow().is_some())
}

#[test]
fn test_handle_call_mut() {
    let mut pool = LocalPool::new();
    let test_value = Rc::new(RefCell::new(None));
    let test_value_r = test_value.clone();

    let counter = Counter::new();

    let future = async move {
        counter.inc().await;
        let v = counter.internal_value().await;
        *(test_value.borrow_mut()) = Some(v);
    };
    pool.spawner().spawn_local(future).unwrap();
    pool.run_until_stalled();
    assert!(test_value_r.borrow().is_some());
    assert!(test_value_r.borrow().unwrap() == 1);
}
