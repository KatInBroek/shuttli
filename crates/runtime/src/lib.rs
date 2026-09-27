//! Bounded workers and a small cooperative executor. No platform policy.
use shuttli_ports::{sync::Result, worker::Port};
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};
pub fn worker<T: ?Sized + Send + 'static>(name: &str, mut backend: Box<T>) -> Result<Port<T>> {
    let (port, jobs) = Port::channel(8);
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            for job in jobs {
                job(backend.as_mut());
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(port)
}
struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
pub fn block_on<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
pub type Task<'a> = Pin<Box<dyn Future<Output = ()> + 'a>>;
pub fn poll_tasks(tasks: &mut Vec<Task<'_>>) {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    tasks.retain_mut(|task| task.as_mut().poll(&mut cx).is_pending());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    #[test]
    fn worker_queue_is_bounded_and_rejected_jobs_complete_without_running() {
        let (port, jobs) = Port::<usize>::channel(1);
        let accepted = port.call(|state| {
            *state += 1;
            Ok(*state)
        });
        let rejected = port.call::<()>(|_| panic!("rejected job must not run"));
        assert!(block_on(rejected).is_err());
        let mut backend = 0;
        jobs.recv().unwrap()(&mut backend);
        assert_eq!(block_on(accepted).unwrap(), 1);
        let abandoned = port.call(|_| Ok(2));
        drop(jobs);
        assert!(block_on(abandoned).is_err());
    }
    #[test]
    fn completion_wakes_a_parked_executor_and_worker_runs_serially() {
        let port = worker("fixture-worker", Box::new(0usize)).unwrap();
        let (started, entered) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let first = port.call(move |n| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            *n += 1;
            Ok(*n)
        });
        entered
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let second = port.call(|n| {
            *n += 1;
            Ok(*n)
        });
        release.send(()).unwrap();
        assert_eq!(block_on(first).unwrap(), 1);
        assert_eq!(block_on(second).unwrap(), 2);
    }
}
