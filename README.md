# Async Event Streams

Library for publishing events for multiple consumers using asynchromous streams

Library provides ```EventStreams<T: 'static + Send + Sync>``` object which translates events of type ```T``` 
to arbitrary number of ```EventStream``` objects, which implements standard ```futures::Stream``` interface

## Usage example

```
use futures::{executor::LocalPool, task::LocalSpawnExt, StreamExt};
use async_events::EventStreams;

let mut pool = LocalPool::new();

let streams = EventStreams::new();
let mut stream = streams.create_event_stream();

let sender_task = async move {
    assert!(streams.count() == 1);
    streams.send_event(42, None).await;
    streams.send_event(451, None).await;
    streams.send_event(1984, None).await;
};

let receiver_task = async move {
    let mut values = Vec::new();
    while let Some(event) = stream.next().await {
        values.push(*event)
    }
    // next() returns none when 'streams' is dropped
    assert!(values == vec![42, 451, 1984]);
};

pool.spawner().spawn_local(sender_task);
pool.spawner().spawn_local(receiver_task);
pool.run();
```