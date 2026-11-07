//! Pausing and stopping a live branch. While a branch is paused its tool calls and gateway model
//! calls wait (they don't fail) until it resumes; once it is stopped they are refused.

use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use tokio::sync::watch;

/// Where a live branch stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Running,
    Paused,
    Stopped,
}

/// What a call that waited is told once the branch has been stopped.
pub const STOPPED: &str = "the run was stopped";

/// The branch was stopped while a call waited, or before it came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stopped;

/// A live branch's flow, and how long it has been paused in all.
pub struct FlowControl {
    flow: watch::Sender<Flow>,
    /// Time paused in the pauses that ended, and when the current one began.
    paused: Mutex<(Duration, Option<Instant>)>,
}

impl Default for FlowControl {
    fn default() -> FlowControl {
        FlowControl {
            flow: watch::channel(Flow::Running).0,
            paused: Mutex::default(),
        }
    }
}

impl FlowControl {
    pub fn get(&self) -> Flow {
        *self.flow.borrow()
    }

    pub fn is_paused(&self) -> bool {
        self.get() == Flow::Paused
    }

    /// Pauses a running branch; false when it wasn't running.
    pub fn pause(&self) -> bool {
        let changed = self.flow.send_if_modified(|f| {
            let running = *f == Flow::Running;
            if running {
                *f = Flow::Paused;
            }
            running
        });
        if changed {
            self.paused.lock().unwrap().1 = Some(Instant::now());
        }
        changed
    }

    /// Resumes a paused branch and releases the calls that wait; how long this pause lasted, or
    /// `None` when it wasn't paused.
    pub fn resume(&self) -> Option<Duration> {
        let changed = self.flow.send_if_modified(|f| {
            let paused = *f == Flow::Paused;
            if paused {
                *f = Flow::Running;
            }
            paused
        });
        changed.then(|| self.end_pause())
    }

    /// Stops the branch: the calls that wait are refused, and so is every later one. False when
    /// it was stopped already.
    pub fn stop(&self) -> bool {
        let changed = self.flow.send_if_modified(|f| {
            let was = *f;
            *f = Flow::Stopped;
            was != Flow::Stopped
        });
        if changed {
            self.end_pause();
        }
        changed
    }

    /// Ends the current pause (if one is open) and counts it; its length.
    fn end_pause(&self) -> Duration {
        let mut p = self.paused.lock().unwrap();
        let this = p.1.take().map_or(Duration::ZERO, |t| t.elapsed());
        p.0 += this;
        this
    }

    /// The time the branch has been paused in all, the current pause included.
    pub fn paused_total(&self) -> Duration {
        let p = self.paused.lock().unwrap();
        p.0 + p.1.map_or(Duration::ZERO, |t| t.elapsed())
    }

    /// Waits while the branch is paused; `Err` once it is stopped.
    pub async fn go(&self) -> Result<(), Stopped> {
        let mut rx = self.flow.subscribe();
        let flow = rx
            .wait_for(|f| *f != Flow::Paused)
            .await
            .map_err(|_| Stopped)?;
        match *flow {
            Flow::Stopped => Err(Stopped),
            _ => Ok(()),
        }
    }

    /// Resolves once the branch is stopped.
    pub async fn stopped(&self) {
        let mut rx = self.flow.subscribe();
        let _ = rx.wait_for(|f| *f == Flow::Stopped).await;
    }

    /// Runs `f` with `limit` of time, not counting the time the branch is paused meanwhile;
    /// `None` when the time ran out first.
    pub async fn timeout<F: std::future::Future>(
        &self,
        limit: Duration,
        f: F,
    ) -> Option<F::Output> {
        let paused_before = self.paused_total();
        let start = tokio::time::Instant::now();
        tokio::pin!(f);
        loop {
            let paused = self.paused_total() - paused_before;
            let deadline = start + limit + paused;
            tokio::select! {
                biased;
                out = &mut f => return Some(out),
                () = tokio::time::sleep_until(deadline) => {
                    if !self.is_paused() && self.paused_total() - paused_before == paused {
                        return None;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn calls_wait_while_paused_and_go_on_resume() {
        let f = std::sync::Arc::new(FlowControl::default());
        assert_eq!(f.go().await, Ok(()));
        assert!(f.pause());
        assert!(!f.pause(), "pausing twice is pausing once");
        let waiting = tokio::spawn({
            let f = f.clone();
            async move { f.go().await }
        });
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(!waiting.is_finished(), "a call waits while paused");
        let lasted = f.resume().unwrap();
        assert!(lasted >= Duration::from_millis(40), "{lasted:?}");
        assert_eq!(waiting.await.unwrap(), Ok(()));
        assert_eq!(
            f.resume(),
            None,
            "resuming a running branch changes nothing"
        );
        assert!(f.paused_total() >= lasted);
    }

    #[tokio::test]
    async fn stop_refuses_waiting_and_later_calls() {
        let f = std::sync::Arc::new(FlowControl::default());
        f.pause();
        let waiting = tokio::spawn({
            let f = f.clone();
            async move { f.go().await }
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(f.stop());
        assert!(!f.stop());
        assert_eq!(waiting.await.unwrap(), Err(Stopped));
        assert_eq!(f.go().await, Err(Stopped));
        assert!(!f.pause(), "a stopped branch can't be paused");
        assert_eq!(f.resume(), None);
        f.stopped().await;
    }

    #[tokio::test]
    async fn a_timeout_leaves_out_paused_time() {
        let f = FlowControl::default();
        let limit = Duration::from_millis(60);
        assert_eq!(f.timeout(limit, std::future::pending::<()>()).await, None);
        f.pause();
        let slow = tokio::time::sleep(Duration::from_millis(120));
        let resume = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            f.resume();
        };
        let (done, ()) = tokio::join!(f.timeout(limit, slow), resume);
        assert_eq!(
            done,
            Some(()),
            "the paused 100 ms don't count against the 60 ms"
        );
    }
}
