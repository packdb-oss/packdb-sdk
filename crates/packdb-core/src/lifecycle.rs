// Copyright 2025 PackDb Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! One shutdown mechanism for PackDB's standalone services.
//!
//! Every service shuts down in the same order:
//!
//! 1. **Stop admitting work.** [`Lifecycle::begin_shutdown`] marks the service
//!    unready, so readiness probes fail, and wakes everything waiting on
//!    [`Lifecycle::shutdown_requested`]: the HTTP server stops accepting
//!    connections and background loops stop starting new iterations.
//! 2. **Drain in-flight requests** within a deadline.
//! 3. **Stop and await background tasks** within a deadline, aborting any that
//!    do not finish.
//! 4. **Close the database pool.**
//! 5. **Flush telemetry and exit**, which the caller does by dropping its
//!    telemetry guard after [`Shutdown::run`] returns.
//!
//! The mechanism is shared; what runs in the background stays with each
//! service. A service registers its own tasks and resources and this module
//! only orders them.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Readiness and the shutdown signal, shared by a service's parts.
#[derive(Clone, Debug)]
pub struct Lifecycle {
    ready: Arc<AtomicBool>,
    stopping: watch::Sender<bool>,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new()
    }
}

impl Lifecycle {
    /// A service that is not ready and not shutting down.
    #[must_use]
    pub fn new() -> Self {
        let (stopping, _) = watch::channel(false);
        Self {
            ready: Arc::new(AtomicBool::new(false)),
            stopping,
        }
    }

    /// Report ready, unless shutdown already began.
    pub fn mark_ready(&self) {
        if !self.is_shutting_down() {
            self.ready.store(true, Ordering::Release);
        }
    }

    /// Whether the service is currently admitting work.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    /// Whether shutdown has begun. Once true it never returns to false.
    #[must_use]
    pub fn is_shutting_down(&self) -> bool {
        *self.stopping.borrow()
    }

    /// Step 1: stop admitting work. Idempotent.
    pub fn begin_shutdown(&self) {
        self.ready.store(false, Ordering::Release);
        self.stopping.send_replace(true);
    }

    /// Resolves once shutdown has begun, immediately if it already has.
    pub fn shutdown_requested(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut receiver = self.stopping.subscribe();
        async move {
            // An error means every sender is gone, which only happens when the
            // service is being torn down anyway.
            let _ = receiver.wait_for(|stopping| *stopping).await;
        }
    }

    /// Begin shutdown on SIGINT or, on Unix, SIGTERM.
    pub fn begin_shutdown_on_signal(&self) -> JoinHandle<()> {
        let lifecycle = self.clone();
        tokio::spawn(async move {
            termination_signal().await;
            tracing::info!("shutdown signal received");
            lifecycle.begin_shutdown();
        })
    }
}

/// Resolves on SIGINT or, on Unix, SIGTERM. A listener that cannot be
/// installed never resolves rather than shutting the service down.
pub async fn termination_signal() {
    let interrupt = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::warn!(%error, "failed to listen for interrupt");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::warn!(%error, "failed to listen for termination");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }
}

type BoxedFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What happened during shutdown, for logs and tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShutdownReport {
    /// Whether serving ended within the drain deadline.
    pub drained: bool,
    /// Background tasks that finished on their own.
    pub tasks_stopped: Vec<&'static str>,
    /// Background tasks aborted at the deadline.
    pub tasks_aborted: Vec<&'static str>,
    /// Whether resources were released.
    pub resources_closed: bool,
}

/// The ordered shutdown of one service.
pub struct Shutdown {
    lifecycle: Lifecycle,
    drain_deadline: Duration,
    task_deadline: Duration,
    tasks: Vec<(&'static str, JoinHandle<()>)>,
    close: Vec<(&'static str, BoxedFuture<()>)>,
}

impl Shutdown {
    /// Build a shutdown sequencer.
    ///
    /// `drain_deadline` bounds how long in-flight work may finish;
    /// `task_deadline` bounds each registered task's own stop.
    #[must_use]
    pub fn new(lifecycle: Lifecycle, drain_deadline: Duration, task_deadline: Duration) -> Self {
        Self {
            lifecycle,
            drain_deadline,
            task_deadline,
            tasks: Vec::new(),
            close: Vec::new(),
        }
    }

    /// A background task that must observe [`Lifecycle::shutdown_requested`]
    /// and return. It is awaited in step 3.
    #[must_use]
    pub fn task(mut self, name: &'static str, handle: JoinHandle<()>) -> Self {
        self.tasks.push((name, handle));
        self
    }

    /// A resource released in step 4, in registration order, such as a pool's
    /// `close()`.
    #[must_use]
    pub fn close(
        mut self,
        name: &'static str,
        close: impl Future<Output = ()> + Send + 'static,
    ) -> Self {
        self.close.push((name, Box::pin(close)));
        self
    }

    /// Serve until shutdown, then complete every step in order.
    ///
    /// `serve` must stop accepting work when [`Lifecycle::shutdown_requested`]
    /// resolves, as axum's graceful shutdown does, and finish once in-flight
    /// work drains. If `serve` ends by itself, for example with an error,
    /// shutdown still begins so nothing is left running.
    pub async fn run<E>(
        self,
        serve: impl Future<Output = Result<(), E>> + Send,
    ) -> (Result<(), E>, ShutdownReport) {
        let Self {
            lifecycle,
            drain_deadline,
            task_deadline,
            tasks,
            close,
        } = self;
        let mut report = ShutdownReport::default();
        tokio::pin!(serve);

        // Serving; a self-ended serve starts shutdown as well.
        let served = tokio::select! {
            result = &mut serve => Some(result),
            () = lifecycle.shutdown_requested() => None,
        };
        lifecycle.begin_shutdown();

        // Step 2: drain.
        let result = match served {
            Some(result) => {
                report.drained = true;
                result
            }
            None => match tokio::time::timeout(drain_deadline, &mut serve).await {
                Ok(result) => {
                    report.drained = true;
                    result
                }
                Err(_) => {
                    tracing::warn!(
                        deadline_ms = drain_deadline.as_millis(),
                        "in-flight requests did not drain before the deadline"
                    );
                    Ok(())
                }
            },
        };

        // Step 3: background tasks, all against one deadline.
        let deadline = tokio::time::Instant::now() + task_deadline;
        for (name, mut handle) in tasks {
            match tokio::time::timeout_at(deadline, &mut handle).await {
                Ok(_) => report.tasks_stopped.push(name),
                Err(_) => {
                    tracing::warn!(task = name, "background task did not stop; aborting");
                    handle.abort();
                    let _ = handle.await;
                    report.tasks_aborted.push(name);
                }
            }
        }

        // Step 4: resources.
        for (name, close) in close {
            close.await;
            tracing::info!(resource = name, "closed");
        }
        report.resources_closed = true;
        tracing::info!(?report, "shutdown complete");
        (result, report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn shutdown_marks_unready_and_is_observable_late() {
        let lifecycle = Lifecycle::new();
        assert!(!lifecycle.is_ready());
        lifecycle.mark_ready();
        assert!(lifecycle.is_ready());
        lifecycle.begin_shutdown();
        lifecycle.begin_shutdown();
        assert!(!lifecycle.is_ready());
        assert!(lifecycle.is_shutting_down());
        // A late waiter resolves at once, and readiness cannot come back.
        lifecycle.shutdown_requested().await;
        lifecycle.mark_ready();
        assert!(!lifecycle.is_ready());
        assert!(!Lifecycle::default().is_shutting_down());
    }

    #[tokio::test]
    async fn steps_run_in_order_after_the_signal() {
        let lifecycle = Lifecycle::new();
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        let worker_lifecycle = lifecycle.clone();
        let worker_order = Arc::clone(&order);
        let worker = tokio::spawn(async move {
            worker_lifecycle.shutdown_requested().await;
            worker_order.lock().expect("lock").push("task stopped");
        });
        let stuck = tokio::spawn(std::future::pending::<()>());

        let serve_lifecycle = lifecycle.clone();
        let serve_order = Arc::clone(&order);
        let serve = async move {
            serve_lifecycle.shutdown_requested().await;
            // An in-flight request finishing after admission stopped.
            tokio::time::sleep(Duration::from_millis(10)).await;
            serve_order.lock().expect("lock").push("drained");
            Ok::<(), std::io::Error>(())
        };
        let close_order = Arc::clone(&order);
        let shutdown = Shutdown::new(
            lifecycle.clone(),
            Duration::from_secs(1),
            Duration::from_millis(50),
        )
        .task("worker", worker)
        .task("stuck", stuck)
        .close("pool", async move {
            close_order.lock().expect("lock").push("pool closed");
        });

        lifecycle.mark_ready();
        let trigger = lifecycle.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            trigger.begin_shutdown();
        });
        let (result, report) = shutdown.run(serve).await;
        assert!(result.is_ok());
        assert!(!lifecycle.is_ready());
        assert_eq!(
            report,
            ShutdownReport {
                drained: true,
                tasks_stopped: vec!["worker"],
                tasks_aborted: vec!["stuck"],
                resources_closed: true,
            }
        );
        let order = order.lock().expect("lock").clone();
        assert_eq!(order.last(), Some(&"pool closed"));
        assert!(order.contains(&"drained") && order.contains(&"task stopped"));
    }

    #[tokio::test]
    async fn a_serve_that_ends_by_itself_still_shuts_everything_down() {
        let lifecycle = Lifecycle::new();
        let closed = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&closed);
        let observer = lifecycle.clone();
        let task = tokio::spawn(async move { observer.shutdown_requested().await });
        let (result, report) =
            Shutdown::new(lifecycle.clone(), Duration::ZERO, Duration::from_secs(1))
                .task("observer", task)
                .close("pool", async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                })
                .run(async { Err::<(), &str>("bind failed") })
                .await;
        assert_eq!(result, Err("bind failed"));
        assert!(lifecycle.is_shutting_down());
        assert_eq!(report.tasks_stopped, vec!["observer"]);
        assert_eq!(closed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_serve_that_does_not_drain_is_abandoned_at_the_deadline() {
        let lifecycle = Lifecycle::new();
        lifecycle.begin_shutdown();
        let (result, report) = Shutdown::new(lifecycle, Duration::from_millis(10), Duration::ZERO)
            .run(std::future::pending::<Result<(), ()>>())
            .await;
        assert!(result.is_ok());
        assert!(!report.drained);
        assert!(report.resources_closed);
    }
}
