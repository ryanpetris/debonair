//! Moves serial data between the USB tasks and the application on the owner task.
use crate::{IN_BYTES, Io, Packet, Status};
use debonair_core::{
    app::{Action, App},
    radio::Radio,
    store::Store,
};

/// How long work such as a restart waits at most for USB to write the output before it, once the
/// application has handed all of it over.
const WRITE_WAIT_MS: u64 = 50;
/// How long the host has to take the last packet after it is handed to the USB controller. The
/// controller sends a packet when the host asks for one, which a host reading the port does every
/// frame.
const TAKE_MS: u64 = 5;

/// Work the application returned, and the times that bound when it runs.
struct Pending {
    action: Action,
    /// When it runs even though output is still waiting.
    deadline: u64,
    /// When it may run: TAKE_MS after the first poll that found everything handed to USB.
    earliest: Option<u64>,
}

pub struct Owner<'a> {
    io: &'a Io,
    session: u64,
    suspended: bool,
    /// Work the application returned, held until USB has written the output before it, such as
    /// the Response to Restart.
    action: Option<Pending>,
    /// A received packet the application has not taken all of, and how much of it it has taken.
    /// While it waits, the USB task stops reading, so the Client's further bytes wait on its side.
    input: Option<(Packet, usize)>,
}

impl<'a> Owner<'a> {
    pub fn new(io: &'a Io) -> Self {
        Self {
            io,
            session: io.status().session,
            suspended: false,
            action: None,
            input: None,
        }
    }

    /// Delivers received packets, runs the application and queues its output. Returns work for the
    /// platform, such as a restart.
    pub fn poll(
        &mut self,
        app: &mut App,
        radio: &mut impl Radio,
        store: &mut impl Store,
        now: u64,
    ) -> Option<Action> {
        let status = self.io.status();
        if status.session != self.session {
            self.session = status.session;
            app.session(status.open);
        }
        // A suspended Client cannot release what it holds.
        if status.suspended && !self.suspended {
            app.release();
        }
        self.suspended = status.suspended;
        self.receive(status, app, radio, store, now);
        self.send(status, app);
        if let Some(action) = app.poll(radio, now) {
            // The application stops waiting while its own output is unsent when the Client is not
            // reading, and then so does this.
            let deadline = if app.has_output() {
                now
            } else {
                now + WRITE_WAIT_MS
            };
            self.action = Some(Pending {
                action,
                deadline,
                earliest: None,
            });
        }
        self.send(status, app);
        let pending = self.action.as_mut()?;
        pending.earliest = if self.io.written() {
            pending.earliest.or(Some(now + TAKE_MS))
        } else {
            None
        };
        if status.open && now < pending.deadline && pending.earliest.is_none_or(|at| now < at) {
            return None;
        }
        self.action.take().map(|pending| pending.action)
    }

    /// Offers received bytes to the application until it stops taking them.
    fn receive(
        &mut self,
        status: Status,
        app: &mut App,
        radio: &mut impl Radio,
        store: &mut impl Store,
        now: u64,
    ) {
        loop {
            let Some((packet, offset)) = self
                .input
                .take()
                .or_else(|| self.io.receive().map(|packet| (packet, 0)))
            else {
                return;
            };
            if packet.session != self.session || !status.open {
                continue;
            }
            let offset = offset + app.receive(&packet.data()[offset..], radio, store, now);
            if offset < packet.length {
                self.input = Some((packet, offset));
                return;
            }
        }
    }

    fn send(&self, status: Status, app: &mut App) {
        let mut buffer = [0; IN_BYTES];
        while status.open && app.has_output() && self.io.can_send() {
            let length = app.output(&mut buffer);
            self.io.send(&buffer[..length]);
        }
    }
}
