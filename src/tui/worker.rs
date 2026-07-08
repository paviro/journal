//! A lazy-spawned background worker over an mpsc channel pair. The blocking
//! network lookups (geocoding, weather) each run on a dedicated thread and reply
//! over a channel the event loop drains every frame — the thread is spawned on
//! the first request, so sessions that never trigger a lookup pay nothing.

use std::{
    sync::mpsc::{Receiver, Sender, channel},
    thread,
};

/// Handle to a background worker resolving `Req` into `Res`. `in_flight` counts
/// dispatched-but-not-yet-drained requests so the event loop can poll faster
/// while a lookup is outstanding.
pub(crate) struct Worker<Req, Res> {
    channels: Option<Channels<Req, Res>>,
    in_flight: usize,
}

struct Channels<Req, Res> {
    requests: Sender<Req>,
    results: Receiver<Res>,
}

impl<Req, Res> Default for Worker<Req, Res> {
    fn default() -> Self {
        Self {
            channels: None,
            in_flight: 0,
        }
    }
}

impl<Req: Send + 'static, Res: Send + 'static> Worker<Req, Res> {
    /// Dispatch a request, spawning the worker thread on the first call. `handler`
    /// resolves each request on that thread; it's a plain `fn` (no captured
    /// state) shared by every call.
    pub(crate) fn request(&mut self, request: Req, handler: fn(Req) -> Res) {
        let channels = self.channels.get_or_insert_with(|| spawn(handler));
        if channels.requests.send(request).is_ok() {
            self.in_flight += 1;
        }
    }

    /// Drain every finished result (empty when the worker was never started).
    pub(crate) fn drain(&mut self) -> Vec<Res> {
        let Some(channels) = &self.channels else {
            return Vec::new();
        };
        let results: Vec<Res> = channels.results.try_iter().collect();
        self.in_flight = self.in_flight.saturating_sub(results.len());
        results
    }

    /// Whether a request is still outstanding.
    pub(crate) fn has_pending(&self) -> bool {
        self.in_flight > 0
    }
}

fn spawn<Req: Send + 'static, Res: Send + 'static>(handler: fn(Req) -> Res) -> Channels<Req, Res> {
    let (request_tx, request_rx) = channel::<Req>();
    let (result_tx, result_rx) = channel::<Res>();
    thread::spawn(move || {
        // Resolve each request in turn — serial by construction. Exits when the
        // request channel is dropped (the app is shutting down).
        while let Ok(request) = request_rx.recv() {
            if result_tx.send(handler(request)).is_err() {
                break;
            }
        }
    });
    Channels {
        requests: request_tx,
        results: result_rx,
    }
}
