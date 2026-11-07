//! Owned, request-scoped work. The UI only polls; shutdown joins on its cleanup thread.
use ascii_painter_video::Cancellation;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::{self, JoinHandle};

pub struct Job<T> {
    pub cancel: Cancellation,
    receiver: Receiver<T>,
    thread: Option<JoinHandle<()>>,
    #[cfg(test)]
    pending_hook: std::sync::Mutex<Option<PendingHook<T>>>,
}

#[cfg(test)]
type PendingHook<T> = Box<dyn FnOnce(&Job<T>) + Send>;

impl<T: Send + 'static> Job<T> {
    pub fn spawn(name: &str, wake: rune::Waker, work: impl FnOnce(Cancellation) -> T + Send + 'static) -> Self {
        let cancel = Cancellation::default();
        let flag = cancel.clone();
        let (sender, receiver) = channel();
        let thread = thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let result = work(flag);
                let _ = sender.send(result);
                wake();
            })
            .expect("start application worker");
        Self {
            cancel,
            receiver,
            thread: Some(thread),
            #[cfg(test)]
            pending_hook: Default::default(),
        }
    }

    pub fn poll(&self) -> Result<T, TryRecvError> {
        // A result is sent before wake() returns. Never let a caller drop/join that
        // still-running thread, and never infer disconnection from a second check.
        if !self.is_finished() {
            #[cfg(test)]
            self.pending_boundary();
            return Err(TryRecvError::Empty);
        }
        self.receiver.try_recv()
    }
    #[cfg(test)]
    pub fn on_pending(&mut self, hook: impl FnOnce(&Self) + Send + 'static) {
        *self.pending_hook.get_mut().unwrap() = Some(Box::new(hook));
    }
    #[cfg(test)]
    fn pending_boundary(&self) {
        let hook = self.pending_hook.lock().unwrap().take();
        if let Some(hook) = hook {
            hook(self);
        }
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl<T> Drop for Job<T> {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_results_are_consumed_exactly_once() {
        let job = Job::spawn("one-result", std::sync::Arc::new(|| {}), |_| 7);
        let start = std::time::Instant::now();
        while !job.is_finished() {
            assert!(start.elapsed() < std::time::Duration::from_secs(5));
            std::thread::yield_now();
        }
        assert_eq!(job.poll(), Ok(7));
        assert_eq!(job.poll(), Err(TryRecvError::Disconnected));
    }
}
