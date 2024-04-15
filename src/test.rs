use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

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
        (&mut this.poll)(future, cx).map(|_| ())
    }
}
