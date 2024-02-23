use std::mem::MaybeUninit;

use crate::{Ctx, Stack};

#[test]
fn test_fibbo() {
    async fn heavy_fibbo(mut ctx: Ctx<'_>, n: usize) -> usize {
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
    async fn deep(mut ctx: Ctx<'_>, n: usize) -> usize {
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
