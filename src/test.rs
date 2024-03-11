use std::mem::MaybeUninit;

use crate::{Stack, Stk};

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

#[test]
fn borrow_lifetime_struct() {
    struct Ref<'a> {
        r: &'a usize,
    }

    async fn root(mut ctx: Stk<'_>) {
        let depth = 100;
        let r = Ref { r: &depth };
        ctx.run(|ctx| go_deep(ctx, r)).await;
    }

    async fn go_deep(mut ctx: Stk<'_>, r: Ref<'_>) {
        let depth = (*r.r) - 1;
        if depth == 0 {
            return;
        }
        let r = Ref { r: &depth };
        ctx.run(|ctx| go_deep(ctx, r)).await
    }

    let mut stack = Stack::new();

    stack.run(root).finish();
}

#[tokio::test]
async fn read_cargo() {
    async fn deep_read(mut ctx: Stk<'_>, n: usize) -> String {
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

    let str = stack.run(|ctx| deep_read(ctx, 200)).finish_async().await;

    assert_eq!(str, include_str!("../Cargo.toml"));
}

#[tokio::test]
async fn read_cargo_spawn() {
    async fn deep_read(mut ctx: Stk<'_>, n: usize) -> String {
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

        let str = stack.run(|ctx| deep_read(ctx, 200)).finish_async().await;

        assert_eq!(str, include_str!("../Cargo.toml"));
    })
    .await
    .unwrap();
}
