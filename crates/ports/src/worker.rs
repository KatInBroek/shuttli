//! Bounded asynchronous access to a synchronous port, without choosing a runtime.
use crate::sync::Result;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, mpsc},
    task::{Context, Poll, Waker},
};
pub type Job<T> = Box<dyn FnOnce(&mut T) + Send>;
pub struct Port<T: ?Sized> {
    sender: mpsc::SyncSender<Job<T>>,
}
impl<T: ?Sized> Clone for Port<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}
impl<T: ?Sized + 'static> Port<T> {
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<Job<T>>) {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        (Self { sender }, receiver)
    }
    pub fn call<R: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut T) -> Result<R> + Send + 'static,
    ) -> Completion<R> {
        let shared = Arc::new(Mutex::new(State {
            result: None,
            waker: None,
        }));
        let completion = Completion(shared.clone());
        let reply = Reply {
            shared,
            completed: false,
        };
        let job = Box::new(move |port: &mut T| {
            reply.finish(operation(port));
        });
        // Dropping a rejected job completes its receiver with an error.
        let _ = self.sender.try_send(job);
        completion
    }
}
struct State<R> {
    result: Option<Result<R>>,
    waker: Option<Waker>,
}
pub struct Completion<R>(Arc<Mutex<State<R>>>);
struct Reply<R> {
    shared: Arc<Mutex<State<R>>>,
    completed: bool,
}
impl<R> Reply<R> {
    fn finish(mut self, result: Result<R>) {
        self.completed = true;
        let mut state = self.shared.lock().expect("completion lock");
        state.result = Some(result);
        if let Some(waker) = state.waker.take() {
            waker.wake();
        }
    }
}
impl<R> Drop for Reply<R> {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let mut state = self.shared.lock().expect("completion lock");
        if state.result.is_none() {
            state.result = Some(Err("port worker unavailable or queue full".into()));
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
    }
}
impl<R> Future for Completion<R> {
    type Output = Result<R>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.lock().expect("completion lock");
        if let Some(result) = state.result.take() {
            Poll::Ready(result)
        } else {
            state.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
