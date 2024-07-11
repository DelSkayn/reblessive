use std::{
    cell::Cell, future::Future, mem::MaybeUninit, pin::Pin, task::Poll, time::Duration, u128,
};

use crate::{
    test::{run_with_stack_size, thread_sleep, ManualPoll, KB, MB, PAGE_SIZE},
    Stack, Stk,
};

#[test]
fn call_not_wrapped() {
    async fn a(ctx: &mut Stk, depth: usize) {
        b(ctx, depth).await
    }

    async fn b(ctx: &mut Stk, depth: usize) {
        if depth == 0 {
            return;
        }
        ctx.run(|ctx| a(ctx, depth - 1)).await
    }

    let mut stack = Stack::new();

    let depth = if cfg!(miri) { 16 } else { 1024 };

    stack.enter(|ctx| a(ctx, depth)).finish();
}

#[test]
fn fibbo() {
    async fn heavy_fibbo(ctx: &mut Stk, n: usize) -> usize {
        // An extra stack allocation to simulate a more complex function.
        let mut ballast: MaybeUninit<[u8; KB]> = std::mem::MaybeUninit::uninit();
        // Make sure the ballast isn't compiled out.
        std::hint::black_box(&mut ballast);

        match n {
            0 => 1,
            1 => 1,
            x => {
                ctx.run(move |ctx| heavy_fibbo(ctx, x - 1)).await
                    + ctx.run(move |ctx| heavy_fibbo(ctx, x - 2)).await
            }
        }
    }
    let mut stack = Stack::new();

    let (v, depth) = if cfg!(miri) { (13, 6) } else { (10946, 20) };

    // run the function to completion on the stack.
    let res = run_with_stack_size(8 * PAGE_SIZE, "fibbo", move || {
        stack.enter(|ctx| heavy_fibbo(ctx, depth)).finish()
    });
    assert_eq!(res, v);
}

#[test]
fn fibbo_stepping() {
    async fn fibbo(ctx: &mut Stk, n: usize) -> usize {
        match n {
            0 => 1,
            1 => 1,
            x => {
                ctx.run(move |ctx| fibbo(ctx, x - 1)).await
                    + ctx.run(move |ctx| fibbo(ctx, x - 2)).await
            }
        }
    }
    let mut stack = Stack::new();

    let (v, depth) = if cfg!(miri) { (13, 6) } else { (10946, 20) };

    // run the function to completion on the stack.
    let mut runner = stack.enter(|ctx| fibbo(ctx, depth));
    loop {
        if let Some(x) = runner.step() {
            assert_eq!(x, v);
            break;
        }
    }
}

#[test]
fn fibbo_stepping_yield() {
    async fn fibbo(ctx: &mut Stk, n: usize) -> usize {
        match n {
            0 => 1,
            1 => 1,
            x => {
                let a = ctx.run(move |ctx| fibbo(ctx, x - 1)).await;
                ctx.yield_now().await;
                a + ctx.run(move |ctx| fibbo(ctx, x - 2)).await
            }
        }
    }
    let mut stack = Stack::new();

    let (v, depth) = if cfg!(miri) { (13, 6) } else { (10946, 20) };

    // run the function to completion on the stack.
    let mut runner = stack.enter(|ctx| fibbo(ctx, depth));
    loop {
        if let Some(x) = runner.step() {
            assert_eq!(x, v);
            break;
        }
    }
}

#[test]
fn fibbo_finish_yield() {
    async fn fibbo(ctx: &mut Stk, n: usize) -> usize {
        match n {
            0 => 1,
            1 => 1,
            x => {
                let a = ctx.run(move |ctx| fibbo(ctx, x - 1)).await;
                ctx.yield_now().await;
                a + ctx.run(move |ctx| fibbo(ctx, x - 2)).await
            }
        }
    }
    let mut stack = Stack::new();

    let (v, depth) = if cfg!(miri) { (13, 6) } else { (10946, 20) };

    // run the function to completion on the stack.
    let res = stack.enter(|ctx| fibbo(ctx, depth)).finish();
    assert_eq!(res, v)
}

#[test]
#[should_panic = "a non-reblessive future was run while running the reblessive runtime as non-async"]
fn not_async() {
    let mut stack = Stack::new();

    stack
        .enter(|_| async { thread_sleep(Duration::from_secs(1)).await })
        .finish();
}

#[test]
fn very_deep() {
    async fn deep(ctx: &mut Stk, n: usize) -> usize {
        // An extra stack allocation to simulate a more complex function.
        let mut ballast: MaybeUninit<[u8; 32 * KB]> = std::mem::MaybeUninit::uninit();
        // Make sure the ballast isn't compiled out.
        std::hint::black_box(&mut ballast);

        if n == 0 {
            return 0xCAFECAFE;
        }

        ctx.run(move |ctx| deep(ctx, n - 1)).await
    }
    let mut stack = Stack::new();

    let depth = if cfg!(miri) { 16 } else { 1024 };

    // run the function to completion on the stack.
    let res = run_with_stack_size(MB, "very_deep", move || {
        stack.enter(|ctx| deep(ctx, depth)).finish()
    });
    assert_eq!(res, 0xCAFECAFE)
}

#[test]
fn deep_sleep() {
    run_with_stack_size(MB, "deep_sleep", || {
        pollster::block_on(async {
            async fn deep(ctx: &mut Stk, n: usize) -> usize {
                // An extra stack allocation to simulate a more complex function.
                let mut ballast: MaybeUninit<[u8; 32 * KB]> = std::mem::MaybeUninit::uninit();
                // Make sure the ballast isn't compiled out.
                std::hint::black_box(&mut ballast);

                if n == 0 {
                    thread_sleep(Duration::from_millis(500)).await;
                    return 0xCAFECAFE;
                }

                ctx.run(move |ctx| deep(ctx, n - 1)).await
            }
            let mut stack = Stack::new();

            let depth = if cfg!(miri) { 16 } else { 1024 };

            // run the function to completion on the stack.
            let res = stack.enter(|ctx| deep(ctx, depth)).finish_async().await;
            assert_eq!(res, 0xCAFECAFE)
        })
    })
}

#[test]
fn mutate_in_future() {
    async fn mutate(ctx: &mut Stk, v: &mut Vec<usize>, depth: usize) {
        v.push(depth);
        if depth != 0 {
            ctx.run(|ctx| mutate(ctx, v, depth - 1)).await
        }
    }

    let mut stack = Stack::new();

    let mut v = Vec::new();
    stack.enter(|ctx| mutate(ctx, &mut v, 1000)).finish();

    for (idx, i) in (0..=1000).rev().enumerate() {
        assert_eq!(v[idx], i)
    }
}

#[test]
fn mutate_created_in_future() {
    async fn root(ctx: &mut Stk) {
        let mut v = Vec::new();

        let depth = if cfg!(miri) { 16 } else { 1024 };

        ctx.run(|ctx| mutate(ctx, &mut v, depth)).await;

        for (idx, i) in (0..=depth).rev().enumerate() {
            assert_eq!(v[idx], i)
        }
    }
    async fn mutate(ctx: &mut Stk, v: &mut Vec<usize>, depth: usize) {
        // An extra stack allocation to simulate a more complex function.
        let mut ballast: MaybeUninit<[u8; 32 * KB]> = std::mem::MaybeUninit::uninit();
        // Make sure the ballast isn't compiled out.
        std::hint::black_box(&mut ballast);
        v.push(depth);
        if depth != 0 {
            ctx.run(|ctx| mutate(ctx, v, depth - 1)).await
        }
    }

    run_with_stack_size(MB, "mutate_created_in_future", || {
        let mut stack = Stack::new();
        stack.enter(root).finish();
    });
}

#[test]
fn borrow_lifetime_struct() {
    struct Ref<'a> {
        r: &'a usize,
    }

    async fn root(ctx: &mut Stk) {
        let depth = 100;
        let r = Ref { r: &depth };
        ctx.run(|ctx| go_deep(ctx, r)).await;
    }

    async fn go_deep(ctx: &mut Stk, r: Ref<'_>) {
        let depth = (*r.r) - 1;
        if depth == 0 {
            return;
        }
        let r = Ref { r: &depth };
        ctx.run(|ctx| go_deep(ctx, r)).await
    }

    let mut stack = Stack::new();

    stack.enter(root).finish();
}

#[test]
fn test_bigger_alignment() {
    use std::u16;

    #[repr(align(32))]
    struct U256(u128, u128);

    struct Rand(u32);

    impl Rand {
        fn new() -> Self {
            Rand(0x194b93c)
        }

        fn next(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }
    }

    async fn count_u16(stk: &mut Stk, rand: &mut Rand, depth: usize) -> usize {
        let mut v = rand.next() as u16;
        if depth == 0 {
            return v as usize;
        }
        // make sure that v is placed onto the stack.
        std::hint::black_box(&mut v);
        let c = match rand.next() % 3 {
            0 => stk.run(|stk| count_u16(stk, rand, depth - 1)).await as u16,
            1 => stk.run(|stk| count_u128(stk, rand, depth - 1)).await as u16,
            2 => stk.run(|stk| count_u256(stk, rand, depth - 1)).await as u16,
            _ => unreachable!(),
        };
        v.wrapping_add(c) as usize
    }

    async fn count_u128(stk: &mut Stk, rand: &mut Rand, depth: usize) -> usize {
        let mut v = rand.next() as u128;
        if depth == 0 {
            return v as usize;
        }
        // make sure that v is placed onto the stack.
        std::hint::black_box(&mut v);
        let c = match rand.next() % 3 {
            0 => stk.run(|stk| count_u16(stk, rand, depth - 1)).await as u128,
            1 => stk.run(|stk| count_u128(stk, rand, depth - 1)).await as u128,
            2 => stk.run(|stk| count_u256(stk, rand, depth - 1)).await as u128,
            _ => unreachable!(),
        };
        v.wrapping_add(c) as usize
    }

    async fn count_u256(stk: &mut Stk, rand: &mut Rand, depth: usize) -> usize {
        let mut v = U256(rand.next() as u128, 120203);
        if depth == 0 {
            return v.0 as usize;
        }
        // make sure that v is placed onto the stack.
        std::hint::black_box(&mut v);
        std::hint::black_box(v.1);
        let c = match rand.next() % 3 {
            0 => stk.run(|stk| count_u16(stk, rand, depth - 1)).await as u128,
            1 => stk.run(|stk| count_u128(stk, rand, depth - 1)).await as u128,
            2 => stk.run(|stk| count_u256(stk, rand, depth - 1)).await as u128,
            _ => unreachable!(),
        };
        v.0.wrapping_add(c) as usize
    }

    let mut rand = Rand::new();
    let mut stack = Stack::new();
    let depth = if cfg!(miri) { 16 } else { 1024 };
    stack
        .enter(|stk| count_u128(stk, &mut rand, depth))
        .finish();
}

// miri doesn't support epoll properly
#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn read_cargo() {
    async fn deep_read(ctx: &mut Stk, n: usize) -> String {
        // smaller ballast since tokio only allocates 2MB for its threads.
        let mut ballast: MaybeUninit<[u8; 1024]> = std::mem::MaybeUninit::uninit();
        std::hint::black_box(&mut ballast);

        if n == 0 {
            tokio::fs::read_to_string("./Cargo.toml").await.unwrap()
        } else {
            ctx.run(|ctx| deep_read(ctx, n - 1)).await
        }
    }

    let mut stack = Stack::new();

    let str = stack.enter(|ctx| deep_read(ctx, 200)).finish_async().await;

    assert_eq!(str, include_str!("../../Cargo.toml"));
}

// miri doesn't support epoll properly
#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn read_cargo_spawn() {
    async fn deep_read(ctx: &mut Stk, n: usize) -> String {
        // smaller ballast since tokio only allocates 2MB for its threads.
        let mut ballast: MaybeUninit<[u8; 1024]> = std::mem::MaybeUninit::uninit();
        std::hint::black_box(&mut ballast);

        if n == 0 {
            tokio::fs::read_to_string("./Cargo.toml").await.unwrap()
        } else {
            ctx.run(|ctx| deep_read(ctx, n - 1)).await
        }
    }

    tokio::spawn(async {
        let mut stack = Stack::new();

        let str = stack.enter(|ctx| deep_read(ctx, 200)).finish_async().await;

        assert_eq!(str, include_str!("../../Cargo.toml"));
    })
    .await
    .unwrap();
}

#[tokio::test]
#[cfg_attr(miri, ignore)]
async fn read_cargo_spawn_step() {
    async fn deep_read(ctx: &mut Stk, n: usize) -> String {
        // smaller ballast since tokio only allocates 2MB for its threads.
        let mut ballast: MaybeUninit<[u8; 1024]> = std::mem::MaybeUninit::uninit();
        std::hint::black_box(&mut ballast);

        ctx.yield_now().await;

        if n == 0 {
            tokio::fs::read_to_string("./Cargo.toml").await.unwrap()
        } else {
            ctx.run(|ctx| deep_read(ctx, n - 1)).await
        }
    }

    tokio::spawn(async {
        let mut stack = Stack::new();
        let mut runner = stack.enter(|ctx| deep_read(ctx, 200));

        loop {
            if let Some(x) = runner.step_async().await {
                assert_eq!(x, include_str!("../../Cargo.toml"));
                break;
            }
        }
    })
    .await
    .unwrap();
}

#[test]
fn poll_once_then_drop() {
    let mut stack = Stack::new();

    async fn other(stk: &mut Stk) {
        stk.yield_now().await;
        stk.yield_now().await;
        stk.yield_now().await;
    }

    async fn inner(stk: &mut Stk) {
        let mut done = false;
        ManualPoll::wrap(stk.run(other), |f, cx| {
            if !done {
                done = true;
                let r = f.poll(cx);
                assert_eq!(r, Poll::Pending);
                r
            } else {
                Poll::Ready(())
            }
        })
        .await
    }

    stack.enter(inner).finish()
}

#[test]
fn poll_after_done() {
    let mut stack = Stack::new();

    async fn other(stk: &mut Stk) {
        stk.yield_now().await;
        stk.yield_now().await;
        stk.yield_now().await;
    }

    async fn inner(stk: &mut Stk) {
        ManualPoll::wrap(stk.run(other), |mut f, cx| match f.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(x) => {
                let _ = f.as_mut().poll(cx);
                let _ = f.as_mut().poll(cx);
                let _ = f.as_mut().poll(cx);
                let _ = f.as_mut().poll(cx);
                Poll::Ready(x)
            }
        })
        .await
    }

    stack.enter(inner).finish()
}

#[test]
fn drop_future() {
    thread_local! {
        static COUNTER: Cell<usize> = const{ Cell::new(0)};
    }

    let mut stack = Stack::new();

    async fn other(_stk: &mut Stk) {
        COUNTER.with(|x| x.set(x.get() + 1))
    }

    async fn inner(stk: &mut Stk) {
        std::mem::drop(stk.yield_now());
        stk.run(other).await;
        std::mem::drop(stk.run(other));
        stk.run(other).await;
    }

    stack.enter(inner).finish();
    assert_eq!(COUNTER.get(), 2)
}

#[test]
fn direct_enter() {
    let mut stack = Stack::new();
    pollster::block_on(async {
        stack
            .enter(|stk| stk.run(|stk| stk.run(|_| thread_sleep(Duration::from_millis(50)))))
            .finish_async()
            .await;
    })
}

#[test]
#[cfg_attr(miri, ignore)]
#[should_panic]
fn forget_runner_and_use_again() {
    thread_local! {
        static COUNTER: Cell<usize> = const{ Cell::new(0)};
    }

    let mut stack = Stack::new();

    async fn other(_stk: &mut Stk) {
        COUNTER.with(|x| x.set(x.get() + 1))
    }

    async fn inner(stk: &mut Stk) {
        std::mem::drop(stk.yield_now());
        stk.run(other).await;
        std::mem::drop(stk.run(other));
        stk.run(other).await;
    }

    let runner = stack.enter(inner);
    std::mem::forget(runner);
    stack.enter(inner).finish();
    assert_eq!(COUNTER.get(), 2)
}

#[test]
#[should_panic(expected = "Invalid stack state, futures are not being evaluated in stack order")]
fn enter_run_after_enter() {
    let mut stack = Stack::new();

    async fn inner(stk: &mut Stk) {
        let a = stk.run(|stk| stk.yield_now());
        let b = Stk::enter_run(|stk| stk.yield_now());

        ManualPoll::wrap((a, b), |f, ctx| {
            let (a, b) = unsafe { f.get_unchecked_mut() };

            let _ = unsafe { Pin::new_unchecked(a) }.poll(ctx);
            let _ = unsafe { Pin::new_unchecked(b) }.poll(ctx);
            panic!("didn't properly panic");
        })
        .await
    }
    stack.enter(inner).finish();
}

#[test]
#[should_panic(expected = "Invalid stack state, futures are not being evaluated in stack order")]
fn enter_after_enter_run() {
    let mut stack = Stack::new();

    async fn inner(stk: &mut Stk) {
        let a = stk.run(|stk| stk.yield_now());
        let b = Stk::enter_run(|stk| stk.yield_now());

        ManualPoll::wrap((a, b), |f, ctx| {
            let (a, b) = unsafe { f.get_unchecked_mut() };

            let _ = unsafe { Pin::new_unchecked(b) }.poll(ctx);
            let _ = unsafe { Pin::new_unchecked(a) }.poll(ctx);
            panic!("didn't properly panic");
        })
        .await
    }
    stack.enter(inner).finish();
}

#[test]
fn cancel_mid_run() {
    async fn fibbo(stk: &mut Stk, f: usize) -> usize {
        match f {
            0 | 1 => 1,
            x => stk.run(|stk| fibbo(stk, x - 1)).await + stk.run(|stk| fibbo(stk, x - 2)).await,
        }
    }

    let mut stack = Stack::new();
    {
        let mut runner = stack.enter(|stk| fibbo(stk, 40));
        for _ in 0..1000 {
            assert!(runner.step().is_none());
        }
    }
}

#[test]
fn drop_mid_run() {
    async fn fibbo(stk: &mut Stk, f: usize) -> usize {
        match f {
            0 | 1 => 1,
            x => stk.run(|stk| fibbo(stk, x - 1)).await + stk.run(|stk| fibbo(stk, x - 2)).await,
        }
    }

    let mut stack = Stack::new();
    let mut runner = stack.enter(|stk| fibbo(stk, 40));
    for _ in 0..1000 {
        assert!(runner.step().is_none());
    }
}

/*
#[test]
fn forget_mid_run() {
    async fn fibbo(stk: &mut Stk, f: usize) -> usize {
        match f {
            0 | 1 => 1,
            x => stk.run(|stk| fibbo(stk, x - 1)).await + stk.run(|stk| fibbo(stk, x - 2)).await,
        }
    }

    let mut stack = Stack::new();
    let mut runner = stack.enter(|stk| fibbo(stk, 40));
    for _ in 0..1000 {
        assert!(runner.step().is_none());
    }
    std::mem::forget(runner);
}
*/
