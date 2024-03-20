use super::TreeStack;
use crate::test::thread_sleep;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};

thread_local! {
    static COUNTER: Cell<usize> = const{ Cell::new(0) };
}

#[test]
fn basic() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();

        let before = Instant::now();
        stack
            .enter(|stk| async move {
                stk.scope(|stk| async move {
                    let futures = (0..10)
                        .map(|_| {
                            stk.run(|_| async {
                                COUNTER.with(|x| x.set(x.get() + 1));
                                thread_sleep(Duration::from_millis(500)).await;
                                COUNTER.with(|x| x.set(x.get() + 1));
                            })
                        })
                        .collect::<Vec<_>>();

                    futures_util::future::join_all(futures).await;
                })
                .await
            })
            .finish()
            .await;

        assert!(before.elapsed() < Duration::from_millis(1000));
        // make sure the futures actually ran.
        assert_eq!(COUNTER.with(|x| x.get()), 20);
    })
}
