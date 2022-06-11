# Async Events

This is a library for publishing/subcribing events for multiple consumers in asynchronous environment.

Below is rough illustration of library usage. The more correct samples are in library documentation and
examples. Also this library is used in experimental Wingows GUI libray [WAG](https://github.com/milyin/wag) 

# Example

This code makes wrappers Background and WBackground for BackgroundImpl object. Internally they are just
Arc\<Rwlock\<BackgroundImpl\>\> and Weak\<Rwlock\<BackgroundImpl\>\> plus tooling for access the Rwlock without blocking
asyncronous job.

```
enum ButtonEvent { Press, Release }

struct Button {
    events: Events
}

impl Button {
    async pub fn press(&mut self) {
        self.send_event(ButtonEvent::Press).await
    }
    pub fn events(&self) -> EventStream<ButtonEvent> {
        self.create_event_stream()
    }
}
```

Below is the code which changes background color when button is pressed

```
let pool = ThreadPool::builder().create().unwrap();
let button = Button::new();
let background = Background::new();

pool.spawn({
    let events = button.events();
    async move {
        // As soon as button is destroyed stream returns None
        while let Some(event) = events.next().await {
            // event has type Event<ButtonEvent>
            match *event.as_ref() {
                ButtonEvent::Pressed => background.set_color(Color::Red).await,
                ButtonEvent::Released => background.set_color(Color::White).await,
            }
        }
    }
});

```
