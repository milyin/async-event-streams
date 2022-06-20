use async_event_streams::EventStreams;
use async_std::future::timeout;
use async_std::task::sleep;
use futures::stream::select;
use futures::{executor::LocalPool, task::LocalSpawnExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn test_send_event() {
    let mut pool = LocalPool::new();
    let subscribers = EventStreams::new();
    let mut numbers = subscribers.create_event_stream();
    pool.spawner()
        .spawn_local(async move {
            subscribers.send_event(1 as usize, None).await;
            subscribers.send_event(2 as usize, None).await;
            subscribers.send_event(3 as usize, None).await;
        })
        .unwrap();
    pool.spawner()
        .spawn_local({
            async move {
                timeout(Duration::from_secs(5), async move {
                    let n1 = numbers.next().await.unwrap();
                    assert!(*n1 == 1);
                    drop(n1);
                    let n2 = numbers.next().await.unwrap();
                    assert!(*n2 == 2);
                    let no_n3 = timeout(Duration::from_millis(1), numbers.next()).await;
                    assert!(no_n3.is_err()); // Sending next event blocked by not dropped previous one
                    drop(n2);
                    let n3 = numbers.next().await.unwrap();
                    assert!(*n3 == 3);
                    drop(n3);
                    // earc is dropped, stream returns None
                    assert!(numbers.next().await.is_none());
                })
                .await
                .unwrap()
            }
        })
        .unwrap();
    pool.run();
}

#[test]
fn test_send_dependent_event() {
    let mut pool = LocalPool::new();
    {
        let source = Arc::new(EventStreams::new());
        let evens = Arc::new(EventStreams::new());
        let odds = Arc::new(EventStreams::new());

        // Send source events - sequence of numbers
        pool.spawner()
            .spawn_local({
                let source = source.clone();
                async move {
                    for n in 0usize..10 {
                        source.send_event(n, None).await;
                    }
                }
            })
            .unwrap();

        // Read events from source and resend only even ones
        pool.spawner()
            .spawn_local({
                let evens = evens.clone();
                let mut src = source.create_event_stream();
                async move {
                    while let Some(en) = src.next().await {
                        let n = *en;
                        // Release source event and skip forward other task to provoke disorder if dependent events does't work
                        // Comment 'send_event' with 'en' parameter, uncomment 'send_event(n, None)' and 'drop_en' to make test fail
                        // TODO: Make this fail part of test
                        // drop(en);
                        sleep(Duration::from_millis(1)).await;
                        if n % 2 == 0 {
                            // evens.send_event(n, None).await;
                            evens.send_event(n, en).await;
                        }
                    }
                }
            })
            .unwrap();

        // Read events from source and resend only odd ones
        pool.spawner()
            .spawn_local({
                let odds = odds.clone();
                let mut src = source.create_event_stream();
                async move {
                    while let Some(en) = src.next().await {
                        let n = *en;
                        // drop(en); -- see comments above
                        if n % 2 != 0 {
                            // odds.send_event(n).await;
                            odds.send_event(n, en).await;
                        }
                    }
                }
            })
            .unwrap();

        pool.spawner()
            .spawn_local({
                let evens = evens.create_event_stream();
                let odds = odds.create_event_stream();
                let mut ns = select(evens, odds);
                async move {
                    timeout(Duration::from_secs(5), async move {
                        let mut expect = 0;
                        while let Some(en) = ns.next().await {
                            let n = *en;
                            assert!(n == expect);
                            expect += 1;
                        }
                    })
                    .await
                    .unwrap()
                }
            })
            .unwrap();
    }
    pool.run();
}
