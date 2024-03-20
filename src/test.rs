use std::time::{Duration, Instant};

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
