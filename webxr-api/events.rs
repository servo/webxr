/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::Frame;
use crate::InputId;
use crate::InputSource;
use crate::SelectEvent;
use crate::Sender;

#[derive(Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub enum Event {
    /// Input source connected
    AddInput(InputSource),
    /// Input source disconnected
    RemoveInput(InputId),
    /// Session ended by device
    SessionEnd,
    /// Session focused/blurred/etc
    VisibilityChange(Visibility),
    /// Selection started / ended
    Select(InputId, SelectEvent, Frame),
}

#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "ipc", derive(serde::Serialize, serde::Deserialize))]
pub enum Visibility {
    /// Session fully displayed to user
    Visible,
    /// Session still visible, but is not the primary focus
    VisibleBlurred,
    /// Session not visible
    Hidden,
}

/// Convenience structure for buffering up events
/// when no event callback has been set
pub enum EventBuffer {
    Buffered(Vec<Event>),
    Sink(Sender<Event>),
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
            EventBuffer::Sink(ref dest) => {
                let _ = dest.send(event);
            }
        }
    }

    pub fn upgrade(&mut self, dest: Sender<Event>) {
        if let EventBuffer::Buffered(ref mut events) = *self {
            for event in events.drain(..) {
                let _ = dest.send(event);
            }
        }
        *self = EventBuffer::Sink(dest)
    }
}
