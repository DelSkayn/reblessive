use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

pub const KB: usize = 1024;
pub const MB: usize = 1024 * KB;
pub const PAGE_SIZE: usize = 4 * KB;

/// A sleep that doesn't rely on epoll and is thus usable in miri.
pub(crate) async fn thread_sleep(duration: Duration) {
    let deadline = Instant::now() + duration;
    let (send, mut recv) = tokio::sync::mpsc::channel::<()>(1);
    std::thread::spawn(move || {
        let sleep_time = deadline.duration_since(Instant::now());
        std::thread::sleep(sleep_time);
        let _ = send.blocking_send(());
    });
    recv.recv().await;
}

/// Struct for lowering from a async block into a manual polling a future.
pub(crate) struct ManualPoll<F, Fn> {
    future: F,
    poll: Fn,
}

impl<F, Fn> ManualPoll<F, Fn>
where
    Fn: FnMut(Pin<&mut F>, &mut Context) -> Poll<()>,
{
    pub fn wrap(future: F, poll: Fn) -> Self {
        ManualPoll { future, poll }
    }
}

impl<F, Fn> Future for ManualPoll<F, Fn>
where
    Fn: FnMut(Pin<&mut F>, &mut Context) -> Poll<()>,
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        let future = unsafe { Pin::new_unchecked(&mut this.future) };
        (this.poll)(future, cx).map(|_| ())
    }
}

pub(crate) fn run_with_stack_size<F, R>(size: usize, name: &'static str, f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    #[cfg(not(miri))]
    {
        std::thread::Builder::new()
            .name(name.to_string())
            .stack_size(size)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }
    #[cfg(miri)]
    {
        f()
    }
}
