use std::sync::{mpsc::channel, Arc};

use async_events::{EventStream, Subscribers};
use async_std::sync::RwLock;
use futures::{
    executor::{LocalPool, ThreadPool},
    join,
    task::{LocalSpawnExt, Spawn, SpawnExt},
    StreamExt,
};

#[derive(Copy, Clone, PartialEq, PartialOrd, Debug)]
enum FizzBuzz {
    Expected,
    Number,
    Fizz,
    Buzz,
    FizzBuzz,
}

struct Sink {
    values: RwLock<Vec<Option<FizzBuzz>>>,
}

impl Sink {
    fn new() -> Self {
        Self {
            values: RwLock::new(Vec::new()),
        }
    }
    async fn set_value(&self, pos: usize, value: FizzBuzz) {
        let mut values = self.values.write().await;
        if pos >= values.len() {
            values.resize(pos, None);
            values.push(Some(value))
        } else if let Some(prev) = values[pos] {
            if value > prev {
                values[pos] = Some(value)
            }
        } else {
            values[pos] = Some(value)
        }
    }
    fn validate(&mut self) -> bool {
        let values = self.values.get_mut();
        for (n, res) in values.iter().enumerate() {
            if let Some(res) = res {
                let expected = match (n % 5 == 0, n % 3 == 0) {
                    (true, true) => FizzBuzz::FizzBuzz,
                    (true, false) => FizzBuzz::Buzz,
                    (false, true) => FizzBuzz::Fizz,
                    (false, false) => FizzBuzz::Number,
                };
                if *res != expected {
                    dbg!(n, *res);
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Clone)]
struct Generator(Arc<Subscribers>);
impl Generator {
    pub fn new() -> Self {
        Generator(Arc::new(Subscribers::new()))
    }
    fn values(&self) -> EventStream<usize> {
        self.0.create_event_stream()
    }
    async fn send_value(&mut self, value: usize) {
        self.0.send_event(value, None).await
    }
}

async fn fizz_buzz_test(pool: impl Spawn + Clone, sink: Arc<Sink>) {
    let mut generator = Generator::new();
    let task_nums = {
        let sink = sink.clone();
        let mut values = generator.values();
        pool.spawn_with_handle(async move {
            while let Some(n) = values.next().await {
                let n = *n.as_ref();
                sink.set_value(n, FizzBuzz::Number).await
            }
        })
        .unwrap()
    };
    let task_fizz = {
        let sink = sink.clone();
        let mut values = generator.values();
        pool.spawn_with_handle(async move {
            while let Some(n) = values.next().await {
                let n = *n.as_ref();
                if n % 3 == 0 {
                    sink.set_value(n, FizzBuzz::Fizz).await
                }
            }
        })
        .unwrap()
    };
    let task_buzz = {
        let tsink = sink.clone();
        let mut values = generator.values();
        pool.spawn_with_handle(async move {
            while let Some(n) = values.next().await {
                let n = *n.as_ref();
                if n % 5 == 0 {
                    tsink.set_value(n, FizzBuzz::Buzz).await
                }
            }
        })
        .unwrap()
    };
    let task_fizzbuzz = {
        let tsink = sink.clone();
        let mut values = generator.values();
        pool.spawn_with_handle(async move {
            while let Some(n) = values.next().await {
                let n = *n.as_ref();
                if n % 5 == 0 && n % 3 == 0 {
                    tsink.set_value(n, FizzBuzz::FizzBuzz).await
                }
            }
        })
        .unwrap()
    };

    pool.spawn({
        let sink = sink.clone();
        async move {
            for n in 1..100 {
                sink.set_value(n, FizzBuzz::Expected).await;
                generator.send_value(n).await;
            }
        }
    })
    .unwrap();

    pool.spawn_with_handle(async move {
        join!(task_nums, task_fizz, task_buzz, task_fizzbuzz);
    })
    .unwrap()
    .await
}

#[test]
fn fizz_buzz_threadpool_async_call() {
    let pool = ThreadPool::builder() /*.pool_size(8)*/
        .create()
        .unwrap();

    let sink = Arc::new(Sink::new());
    let handle = fizz_buzz_test(pool.clone(), sink.clone());

    let (tx, rx) = channel();
    pool.spawn_ok(async move {
        join!(handle);
        tx.send(()).unwrap();
    });
    let _ = rx.recv().unwrap();
    assert!(Arc::try_unwrap(sink).ok().unwrap().validate());
}

#[test]
fn fizz_buzz_locapool_sync_call() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();
    let sink = Arc::new(Sink::new());
    spawner
        .spawn_local(fizz_buzz_test(spawner.clone(), sink.clone()))
        .unwrap();
    pool.run();
    assert!(Arc::try_unwrap(sink).ok().unwrap().validate());
}
