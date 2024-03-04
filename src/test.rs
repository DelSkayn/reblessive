use std::mem::MaybeUninit;

use crate::{Stk, Stack};

#[test]
fn call_not_wrapped() {
    async fn a(ctx: &mut Stk<'_>, depth: usize) {
        b(ctx, depth).await
    }

    async fn b(ctx: &mut Stk<'_>, depth: usize) {
        if depth == 0 {
            return;
        }
        ctx.run(|mut ctx| async move { a(&mut ctx, depth - 1).await })
            .await
    }

    let mut stack = Stack::new();

    #[cfg(miri)]
    let depth = 10;
    #[cfg(not(miri))]
    let depth = 1000;
    stack
        .run(|mut ctx| async move { a(&mut ctx, depth).await })
        .finish();
}

#[test]
fn fibbo() {
    async fn heavy_fibbo(mut ctx: Stk<'_>, n: usize) -> usize {
        // An extra stack allocation to simulate a more complex function.
        let mut ballast: MaybeUninit<[u8; 1024 * 128]> = std::mem::MaybeUninit::uninit();
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

    #[cfg(miri)]
    let (v, depth) = (13, 6);
    #[cfg(not(miri))]
    let (v, depth) = (10946, 20);

    // run the function to completion on the stack.
    let res = stack.run(|ctx| heavy_fibbo(ctx, depth)).finish();
    assert_eq!(res, v);
}

#[test]
fn very_deep() {
    async fn deep(mut ctx: Stk<'_>, n: usize) -> usize {
        // An extra stack allocation to simulate a more complex function.
        let mut ballast: MaybeUninit<[u8; 1024 * 128]> = std::mem::MaybeUninit::uninit();
        // Make sure the ballast isn't compiled out.
        std::hint::black_box(&mut ballast);

        if n == 0 {
            return 0xCAFECAFE;
        }

        ctx.run(move |ctx| deep(ctx, n - 1)).await
    }
    let mut stack = Stack::new();

    #[cfg(miri)]
    let depth = 10;
    #[cfg(not(miri))]
    let depth = 1000;

    // run the function to completion on the stack.
    let res = stack.run(|ctx| deep(ctx, depth)).finish();
    assert_eq!(res, 0xCAFECAFE)
}

#[test]
fn mutate_in_future() {
    async fn mutate(mut ctx: Stk<'_>, v: &mut Vec<usize>, depth: usize) {
        v.push(depth);
        if depth != 0 {
            ctx.run(|ctx| mutate(ctx, v, depth - 1)).await
        }
    }

    let mut stack = Stack::new();

    let mut v = Vec::new();
    stack.run(|ctx| mutate(ctx, &mut v, 1000)).finish();

    for (idx, i) in (0..=1000).rev().enumerate() {
        assert_eq!(v[idx], i)
    }
}

#[test]
fn mutate_created_in_future() {
    async fn root(mut ctx: Stk<'_>) {
        let mut v = Vec::new();
        ctx.run(|ctx| mutate(ctx, &mut v, 1000)).await;

        for (idx, i) in (0..=1000).rev().enumerate() {
            assert_eq!(v[idx], i)
        }
    }
    async fn mutate(mut ctx: Stk<'_>, v: &mut Vec<usize>, depth: usize) {
        v.push(depth);
        if depth != 0 {
            ctx.run(|ctx| mutate(ctx, v, depth - 1)).await
        }
    }

    let mut stack = Stack::new();

    stack.run(root).finish();
}
