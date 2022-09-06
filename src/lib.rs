//!
//! Library for publishing events for multiple consumers using asynchromous streams
//!
//! Library provides [EventStreams<T: 'static + Send + Sync>](EventStreams) object which translates events of type ```T```
//! to arbitrary number of [EventStream] objects, which implements standard [futures::Stream] interface
//!
//! # Usage sample
//!
//! ```
//! use futures::{executor::LocalPool, task::LocalSpawnExt, StreamExt};
//! use async_event_streams::EventStreams;
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
//!         values.push(*event);
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
//! When event is put to [EventStreams] it becomes immediately available for all [EventStream] objects, created by this ```EventStreams```.
//! Events comes from each stream exactly in order as they being sent. Streams are independent from each other, so if event is put into streams,
//! nothing can stop receiver to read it.
//!
//! Since reveivers works in asynchronous environment it's possible that streams are emptied unevenly. I.e. if events 1,2,3,4,5 put to [EventStreams],
//! one [EventStream] subscriber could process all 5 events while another is still waiting for first.
//!
//! Sometimes it's undesirable. So the mechanism to guarantee that all events '1' are handled before sending event '2' is implemented.
//!
//! To achieve this the [send_event](EventStreams::send_event) function returns future [SentEvent]. Each [EventStream] instance receives clone
//! of [Event<T>](Event) object. When all these clones are dropped the ```SentEvent``` future is released. This guarantees that '2' is sent only
//! when '1' has been processed by all subscribers.
//!
//! If such blocking is not necessary, the [post_event](EventStreams::post_event) can be used instead.
//!
//! # Dependent Events
//!
//! Received events may cause firing new events. For example button's mouse click handler is sending button press events.
//! It may be important to guarantee that button click events are not handled in order different than mouse clicks order.
//!
//! For example consider two buttons A and B. Click C1 causes button A send press
//! P1, click C2 causes button B send press P2. It's guaranteed that P2 is *sent* after P1 (because P1 is reaction to C1,
//! P2 is reaction to C2, and both C1 and C2 comes from same ```send_event```).  But there is still no guarantee that P2 is *processed* after P1,
//! because P1 and P2 are sent by different ```send_event```s so the blocking mechanism decribed above doesn't help.
//!
//! This may be inappropriate. For example: user clicks "Apply" button and then "Close" button in the dialog. But press event from "Close"
//! button comes earlier than from "Apply". "Close" handler destroys the dialog, "Apply" is not processed, user's data is lost.
//!
//! To avoid this the [send_event](EventStreams::send_event) and [post_event](EventStreams::post_event) have
//! the additional optional parameter ```source``` - event which was the cause of the sent one. Reference to this 'source' event is saved inside [Event] wrapper of new event
//! and therefore source ```send_event``` is blocked until all derived events are dropped. So sending second click event in example above is delayed until
//! "Apply" handler, which holds fisrst click event, finishes.
//!
//! # Event sources, sinks and pipes
//!
//! There are typical repeating operations with event streams. Object may generate events of different types ([EventSource])
//! and react to events ([EventSink]). Connecting event source to event sink can be performed by spawing asynchronous task with
//! [spawn_event_pipe]
//!

mod event;
mod event_queue;
mod event_stream;
mod event_streams;
mod pipes;

pub use event::{Event, EventBox};
pub use event_stream::EventStream;
pub use event_streams::{EventStreams, SentEvent};
pub use pipes::{
    spawn_event_pipe, spawn_event_pipe_with_handle, EventSink, EventSinkExt, EventSource,
};
