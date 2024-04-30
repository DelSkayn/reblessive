use futures_util::future::join_all;

use super::{Stk, TreeStack};
use crate::{
    test::{thread_sleep, ManualPoll},
    tree::stk::ScopeStk,
};
use std::{
    cell::Cell,
    future::Future,
    mem::MaybeUninit,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    task::Poll,
    time::{Duration, Instant},
};

thread_local! {
    static COUNTER: Cell<usize> = const{ Cell::new(0) };
}

async fn fanout<'a, F, Fut, R>(stk: &'a mut Stk, count: usize, f: F) -> Vec<R>
where
    F: Fn(&'a mut Stk) -> Fut + 'a,
    Fut: Future<Output = R> + 'a,
    R: 'a,
{
    stk.scope(|stk| async move {
        let futures = (0..count)
            .map(|_| stk.run(|stk| f(stk)))
            .collect::<Vec<_>>();

        futures_util::future::join_all(futures).await
    })
    .await
}

#[test]
fn basic() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();

        let before = Instant::now();
        stack
            .enter(|stk| async move {
                fanout::<_, _, ()>(stk, 10, |_| async {
                    COUNTER.with(|x| x.set(x.get() + 1));
                    thread_sleep(Duration::from_millis(500)).await;
                    COUNTER.with(|x| x.set(x.get() + 1));
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

#[test]
fn two_depth() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();

        let before = Instant::now();
        stack
            .enter(|stk| {
                fanout(stk, 4, |stk| async move {
                    fanout(stk, 4, |_| async move {
                        COUNTER.with(|x| x.set(x.get() + 1));
                        thread_sleep(Duration::from_millis(500)).await;
                        COUNTER.with(|x| x.set(x.get() + 1));
                    })
                    .await
                })
            })
            .finish()
            .await;

        assert!(before.elapsed() < Duration::from_millis(2000));
        // make sure the futures actually ran.
        assert_eq!(COUNTER.with(|x| x.get()), 32);
    })
}

#[test]
fn basic_then_deep() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();

        async fn go_deep(stk: &mut Stk, depth: usize) {
            if depth == 0 {
                COUNTER.with(|x| x.set(x.get() + 1));
                thread_sleep(Duration::from_millis(500)).await;
                COUNTER.with(|x| x.set(x.get() + 1));
            } else {
                stk.run(|stk| go_deep(stk, depth - 1)).await
            }
        }

        let before = Instant::now();
        stack
            .enter(|stk| {
                fanout(stk, 10, |stk| async move {
                    go_deep(stk, 10).await;
                })
            })
            .finish()
            .await;

        assert!(before.elapsed() < Duration::from_millis(2000));
        // make sure the futures actually ran.
        assert_eq!(COUNTER.with(|x| x.get()), 20);
    })
}

#[test]
fn two_depth_step() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();

        let before = Instant::now();
        let mut runner = stack.enter(|stk| {
            fanout(stk, 4, |stk| async move {
                fanout(stk, 4, |_| async move {
                    COUNTER.with(|x| x.set(x.get() + 1));
                    thread_sleep(Duration::from_millis(500)).await;
                    COUNTER.with(|x| x.set(x.get() + 1));
                })
                .await
            })
        });

        while runner.step().await.is_none() {}

        assert!(before.elapsed() < Duration::from_millis(2000));
        assert_eq!(COUNTER.with(|x| x.get()), 32);

        // make sure the futures actually ran.
    })
}

#[test]
fn deep_fanout_no_overflow() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();

        let depth = if cfg!(miri) { 10 } else { 1000 };

        async fn go_deep(stk: &mut Stk, deep: usize) -> String {
            // An extra stack allocation to simulate a more complex function.
            let mut ballast: MaybeUninit<[u8; 1024 * 128]> = std::mem::MaybeUninit::uninit();

            let res = if deep != 0 {
                fanout(stk, 1, move |stk| go_deep(stk, deep - 1))
                    .await
                    .into_iter()
                    .next()
                    .unwrap()
            } else {
                "Foo".to_owned()
            };

            // Make sure the ballast isn't compiled out.
            std::hint::black_box(&mut ballast);

            res
        }

        let res = stack.enter(|stk| go_deep(stk, depth)).finish().await;
        assert_eq!(res, "Foo")
    })
}

#[test]
fn deep_no_overflow() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();

        let depth = if cfg!(miri) { 10 } else { 1000 };

        async fn go_deep(stk: &ScopeStk, deep: usize) -> String {
            // An extra stack allocation to simulate a more complex function.
            let mut ballast: MaybeUninit<[u8; 1024 * 128]> = std::mem::MaybeUninit::uninit();

            let res = if deep != 0 {
                stk.run(|stk| stk.scope(|stk| go_deep(stk, deep - 1))).await
            } else {
                "Foo".to_owned()
            };

            // Make sure the ballast isn't compiled out.
            std::hint::black_box(&mut ballast);

            res
        }

        let res = stack
            .enter(|stk| async { stk.scope(|stk| go_deep(stk, depth)).await })
            .finish()
            .await;
        assert_eq!(res, "Foo")
    })
}

#[test]
fn cancel_scope_future() {
    pollster::block_on(async {
        let mut stack = TreeStack::new();
        stack
            .enter(|stk| async {
                let scope = stk.scope(|stk| async {
                    stk.run(|stk| async {
                        stk.yield_now().await;
                        // future should be canceled before this point
                        panic!();
                    })
                    .await
                });

                let mut first = true;
                ManualPoll::wrap(scope, move |future, ctx| {
                    if first {
                        first = false;

                        assert!(matches!(future.poll(ctx), Poll::Pending));
                        Poll::Pending
                    } else {
                        // first poll done, return read so we can cancel it.
                        Poll::Ready(())
                    }
                });
                stk.yield_now().await;
            })
            .finish()
            .await;
    });
}

#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn tokio_sleep_depth() {
    COUNTER.with(|x| x.set(0));
    let mut stack = TreeStack::new();

    let before = Instant::now();
    stack
        .enter(|stk| {
            fanout(stk, 4, |stk| async move {
                fanout(stk, 4, |_| async move {
                    COUNTER.with(|x| x.set(x.get() + 1));
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    COUNTER.with(|x| x.set(x.get() + 1));
                })
                .await
            })
        })
        .finish()
        .await;

    assert!(before.elapsed() < Duration::from_millis(2000));
    // make sure the futures actually ran.
    assert_eq!(COUNTER.with(|x| x.get()), 32);
}

#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn read_files() {
    static OPEN_COUNT: AtomicUsize = AtomicUsize::new(0);
    const MAX_OPEN: usize = 100;

    let mut stack = TreeStack::new();

    async fn read_dir(stk: &ScopeStk, dir: PathBuf) -> String {
        let mut dir = tokio::fs::read_dir(dir).await.unwrap();
        let mut r = Vec::new();
        let mut buf = String::new();
        while let Some(entry) = dir.next_entry().await.unwrap() {
            let path = entry.path();
            let kind = entry.file_type().await.unwrap();
            if kind.is_dir() {
                if OPEN_COUNT.load(Ordering::Relaxed) > MAX_OPEN {
                    let str = stk.run(|stk| stk.scope(|stk| read_dir(stk, path))).await;
                    buf.push_str(&str);
                } else {
                    OPEN_COUNT.fetch_add(1, Ordering::Relaxed);
                    let f = stk.run(|stk| async {
                        let r = stk.scope(|stk| read_dir(stk, path)).await;
                        OPEN_COUNT.fetch_sub(1, Ordering::Relaxed);
                        r
                    });
                    r.push(f)
                }
            } else {
                if OPEN_COUNT.load(Ordering::Relaxed) > MAX_OPEN {
                    let str = stk
                        .run(|_| async {
                            tokio::fs::read_to_string(path).await.unwrap_or_default()
                        })
                        .await;
                    buf.push_str(&str);
                } else {
                    OPEN_COUNT.fetch_add(1, Ordering::Relaxed);
                    let f = stk.run(|_| async {
                        let r = tokio::fs::read_to_string(path).await.unwrap_or_default();
                        OPEN_COUNT.fetch_sub(1, Ordering::Relaxed);
                        r
                    });
                    r.push(f)
                }
            }
        }
        let mut str = join_all(r).await.join("\n=========\n");
        str.push_str(&buf);
        str
    }

    stack
        .enter(|stk| async {
            stk.scope(|stk| read_dir(stk, Path::new("./").to_path_buf()))
                .await
        })
        .finish()
        .await;

    //println!("{}", full_text);
}

#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn read_files_stepping() {
    static OPEN_COUNT: AtomicUsize = AtomicUsize::new(0);
    const MAX_OPEN: usize = 100;

    let mut stack = TreeStack::new();

    async fn read_dir(stk: &ScopeStk, dir: PathBuf) -> String {
        let mut dir = tokio::fs::read_dir(dir).await.unwrap();
        let mut r = Vec::new();
        let mut buf = String::new();
        while let Some(entry) = dir.next_entry().await.unwrap() {
            let path = entry.path();
            let kind = entry.file_type().await.unwrap();
            if kind.is_dir() {
                if OPEN_COUNT.load(Ordering::Relaxed) > MAX_OPEN {
                    let str = stk.run(|stk| stk.scope(|stk| read_dir(stk, path))).await;
                    buf.push_str(&str);
                } else {
                    OPEN_COUNT.fetch_add(1, Ordering::Relaxed);
                    let f = stk.run(|stk| async {
                        let r = stk.scope(|stk| read_dir(stk, path)).await;
                        OPEN_COUNT.fetch_sub(1, Ordering::Relaxed);
                        r
                    });
                    r.push(f)
                }
            } else {
                if OPEN_COUNT.load(Ordering::Relaxed) > MAX_OPEN {
                    let str = stk
                        .run(|_| async {
                            tokio::fs::read_to_string(path).await.unwrap_or_default()
                        })
                        .await;
                    buf.push_str(&str);
                } else {
                    OPEN_COUNT.fetch_add(1, Ordering::Relaxed);
                    let f = stk.run(|_| async {
                        let r = tokio::fs::read_to_string(path).await.unwrap_or_default();
                        OPEN_COUNT.fetch_sub(1, Ordering::Relaxed);
                        r
                    });
                    r.push(f)
                }
            }
        }
        let mut str = join_all(r).await.join("\n=========\n");
        str.push_str(&buf);
        str
    }

    let mut runner = stack.enter(|stk| async {
        stk.scope(|stk| read_dir(stk, Path::new("./").to_path_buf()))
            .await
    });

    loop {
        if let Some(_) = runner.step().await {
            //println!("{}", x);
            break;
        }
    }
}
