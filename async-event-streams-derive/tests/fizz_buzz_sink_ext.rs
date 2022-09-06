use std::{
    borrow::Cow,
    sync::{mpsc::channel, Arc},
};

use async_event_streams_derive::EventSink;

use async_event_streams::{
    spawn_event_pipe, spawn_event_pipe_with_handle, EventBox, EventSink, EventSinkExt, EventSource,
    EventStream, EventStreams,
};
use async_std::sync::RwLock;
use async_trait::async_trait;
use futures::{
    executor::{LocalPool, ThreadPool},
    join,
    task::{LocalSpawnExt, Spawn, SpawnExt},
};

#[derive(Copy, Clone, PartialEq, PartialOrd, Debug)]
enum FizzBuzz {
    Number,
    Fizz,
    Buzz,
    FizzBuzz,
}

#[derive(Copy, Clone, PartialEq, PartialOrd, Debug)]
struct N(usize);

#[derive(EventSink)]
#[event_sink(event=FizzBuzz)]
struct Sink {
    values: RwLock<Vec<FizzBuzz>>,
}

#[async_trait]
impl EventSinkExt<FizzBuzz> for Sink {
    type Error = ();
    async fn on_event<'a>(
        &'a self,
        event: Cow<'a, FizzBuzz>,
        _: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        self.values.write().await.push(*event);
        Ok(())
    }
}

// Just to demostrate that multiple implemetations of EventSink is possible and
// EventSink and EventSinkExt can be mixed
#[async_trait]
impl EventSink<N> for Sink {
    type Error = ();
    async fn on_event_owned(&self, _: N, _: Option<Arc<EventBox>>) -> Result<(), Self::Error> {
        todo!()
    }
    async fn on_event_ref(&self, _: &N, _: Option<Arc<EventBox>>) -> Result<(), Self::Error> {
        todo!()
    }
}

impl Sink {
    fn new() -> Self {
        Self {
            values: RwLock::new(Vec::new()),
        }
    }
    fn validate(&mut self, count: usize) -> bool {
        let values = self.values.get_mut();
        dbg!(values.len());
        assert!(values.len() == count);
        for (n, res) in values.iter().enumerate() {
            let expected = match (n % 5 == 0, n % 3 == 0) {
                (true, true) => FizzBuzz::FizzBuzz,
                (true, false) => FizzBuzz::Buzz,
                (false, true) => FizzBuzz::Fizz,
                (false, false) => FizzBuzz::Number,
            };
            if *res != expected {
                return false;
            }
        }
        true
    }
}

struct Generator(EventStreams<N>);
impl Generator {
    pub fn new() -> Self {
        Generator(EventStreams::new())
    }
    pub async fn run(&self, count: usize) {
        for n in 0..count {
            self.0.send_event(N(n), None).await
        }
    }
}

impl EventSource<N> for Generator {
    fn event_stream(&self) -> EventStream<N> {
        self.0.create_event_stream()
    }
}

#[derive(EventSink)]
#[event_sink(event=N)]
struct Filter {
    mode: FizzBuzz,
    stream: EventStreams<FizzBuzz>,
}

impl Filter {
    fn new(mode: FizzBuzz) -> Self {
        Self {
            mode,
            stream: EventStreams::new(),
        }
    }
}

impl EventSource<FizzBuzz> for Filter {
    fn event_stream(&self) -> EventStream<FizzBuzz> {
        self.stream.create_event_stream()
    }
}

#[async_trait]
impl EventSinkExt<N> for Filter {
    type Error = ();
    async fn on_event<'a>(
        &'a self,
        event: Cow<'a, N>,
        source: Option<Arc<EventBox>>,
    ) -> Result<(), Self::Error> {
        let n = event.0;
        let mode = match (n % 5 == 0, n % 3 == 0) {
            (true, true) => FizzBuzz::FizzBuzz,
            (true, false) => FizzBuzz::Buzz,
            (false, true) => FizzBuzz::Fizz,
            (false, false) => FizzBuzz::Number,
        };
        if self.mode == mode {
            self.stream.send_event(mode, source).await;
        }
        Ok(())
    }
}

async fn fizz_buzz_test(spawner: impl Spawn, sink: Arc<Sink>, count: usize) {
    let generator = Generator::new();
    let fizz = Arc::new(Filter::new(FizzBuzz::Fizz));
    let buzz = Arc::new(Filter::new(FizzBuzz::Buzz));
    let fizzbuzz = Arc::new(Filter::new(FizzBuzz::FizzBuzz));
    let number = Arc::new(Filter::new(FizzBuzz::Number));

    spawn_event_pipe(&spawner, &generator, fizz.clone(), |_| panic!()).unwrap();
    spawn_event_pipe(&spawner, &generator, buzz.clone(), |_| panic!()).unwrap();
    spawn_event_pipe(&spawner, &generator, fizzbuzz.clone(), |_| panic!()).unwrap();
    spawn_event_pipe(&spawner, &generator, number.clone(), |_| panic!()).unwrap();

    let task_fizz =
        spawn_event_pipe_with_handle(&spawner, &*fizz, sink.clone(), |_| panic!()).unwrap();
    let task_buzz =
        spawn_event_pipe_with_handle(&spawner, &*buzz, sink.clone(), |_| panic!()).unwrap();
    let task_fizzbuzz =
        spawn_event_pipe_with_handle(&spawner, &*fizzbuzz, sink.clone(), |_| panic!()).unwrap();
    let task_nums =
        spawn_event_pipe_with_handle(&spawner, &*number, sink.clone(), |_| panic!()).unwrap();

    let task_generator = spawner
        .spawn_with_handle(async move {
            generator.run(count).await;
        })
        .unwrap();

    spawner
        .spawn_with_handle(async move {
            // Make sure that all numebers are sent
            join!(task_generator);
            drop(fizz);
            drop(buzz);
            drop(fizzbuzz);
            drop(number);
            join!(task_nums, task_fizz, task_buzz, task_fizzbuzz);
        })
        .unwrap()
        .await
}

#[test]
fn fizz_buzz_pipes_threadpool_async_call() {
    let pool = ThreadPool::builder() /*.pool_size(8)*/
        .create()
        .unwrap();

    let count = 100;
    let sink = Arc::new(Sink::new());
    let handle = fizz_buzz_test(pool.clone(), sink.clone(), count);
    let (tx, rx) = channel();
    pool.spawn_ok(async move {
        join!(handle);
        tx.send(()).unwrap();
    });
    let _ = rx.recv().unwrap();
    assert!(Arc::try_unwrap(sink).ok().unwrap().validate(count));
}

#[test]
fn fizz_buzz_pipes_locapool_sync_call() {
    let count = 100;
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();
    let sink = Arc::new(Sink::new());
    spawner
        .spawn_local(fizz_buzz_test(spawner.clone(), sink.clone(), count))
        .unwrap();
    pool.run();
    assert!(Arc::try_unwrap(sink).ok().unwrap().validate(count));
}
