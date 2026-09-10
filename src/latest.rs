//! A bounded mailbox: producers replace the pending value instead of waiting
//! for a consumer that may be doing I/O.

use crossbeam::channel::{self, TrySendError};
use std::sync::{Arc, Mutex};

pub struct Sender<T> {
    pending: Arc<Mutex<Option<T>>>,
    wake: channel::Sender<()>,
}

pub struct Receiver<T> {
    pending: Arc<Mutex<Option<T>>>,
    wake: channel::Receiver<()>,
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let pending = Arc::new(Mutex::new(None));
    let (tx, rx) = channel::bounded(1);
    (
        Sender {
            pending: pending.clone(),
            wake: tx,
        },
        Receiver { pending, wake: rx },
    )
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            pending: self.pending.clone(),
            wake: self.wake.clone(),
        }
    }
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), channel::SendError<()>> {
        *self.pending.lock().expect("mailbox poisoned") = Some(value);
        match self.wake.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Disconnected(())) => Err(channel::SendError(())),
        }
    }
}

impl<T> Receiver<T> {
    pub fn ready(&self) -> &channel::Receiver<()> {
        &self.wake
    }

    pub fn take(&self) -> Option<T> {
        self.pending.lock().expect("mailbox poisoned").take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_consumer_receives_latest_value_without_blocking_producers() {
        let (tx, rx) = channel();
        for n in 0..10_000 {
            tx.send(n).unwrap();
        }
        rx.ready().recv().unwrap();
        assert_eq!(rx.take(), Some(9_999));
        assert!(rx.take().is_none());
        tx.send(10_000).unwrap();
        rx.ready().recv().unwrap();
        assert_eq!(rx.take(), Some(10_000));
    }

    #[test]
    fn disconnection_is_reported_in_both_directions() {
        let (tx, rx) = channel::<()>();
        drop(rx);
        assert!(tx.send(()).is_err());
        let (tx, rx) = channel::<()>();
        drop(tx);
        assert!(rx.ready().recv().is_err());
    }
}
