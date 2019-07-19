use crate::InputId;
use crate::InputSource;

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub enum Event {
    /// Input source connected
    AddInput(InputSource),
    /// Input source disconnected
    RemoveInput(InputId),
    /// Session ended by device
    SessionEnd,
}

#[cfg_attr(feature = "ipc", typetag::serde)]
pub trait EventCallback: 'static + Send {
    fn callback(&mut self, event: Event);
}

/// Convenience structure for buffering up events
/// when no event callback has been set
pub enum EventBuffer {
    Buffered(Vec<Event>),
    Sink(Box<dyn EventCallback>),
}

impl Default for EventBuffer {
    fn default() -> Self {
        EventBuffer::Buffered(vec![])
    }
}

impl EventBuffer {
    pub fn callback(&mut self, event: Event) {
        match *self {
            EventBuffer::Buffered(ref mut events) => events.push(event),
            EventBuffer::Sink(ref mut sink) => sink.callback(event),
        }
    }

    pub fn upgrade(&mut self, mut sink: Box<dyn EventCallback>) {
        if let EventBuffer::Buffered(ref mut events) = *self {
            for event in events.drain(..) {
                sink.callback(event)
            }
        }
        *self = EventBuffer::Sink(sink)
    }
}
