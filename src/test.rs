use std::mem::MaybeUninit;

use crate::{Stack, Stk};

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

    #[cfg(miri)]
    let depth = 10;
    #[cfg(not(miri))]
    let depth = 1000;
    stack.enter(|ctx| a(ctx, depth)).finish();
}

#[test]
fn fibbo() {
    async fn heavy_fibbo(ctx: &mut Stk, n: usize) -> usize {
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
    let res = stack.enter(|ctx| heavy_fibbo(ctx, depth)).finish();
    assert_eq!(res, v);
}

#[test]
fn very_deep() {
    async fn deep(ctx: &mut Stk, n: usize) -> usize {
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
    let res = stack.enter(|ctx| deep(ctx, depth)).finish();
    assert_eq!(res, 0xCAFECAFE)
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
        ctx.run(|ctx| mutate(ctx, &mut v, 1000)).await;

        for (idx, i) in (0..=1000).rev().enumerate() {
            assert_eq!(v[idx], i)
        }
    }
    async fn mutate(ctx: &mut Stk, v: &mut Vec<usize>, depth: usize) {
        v.push(depth);
        if depth != 0 {
            ctx.run(|ctx| mutate(ctx, v, depth - 1)).await
        }
    }

    let mut stack = Stack::new();

    stack.enter(root).finish();
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
        let v = rand.next() as u16;
        if depth == 0 {
            return v as usize;
        }
        let c = if (rand.next() & 1) == 0 {
            stk.run(|stk| count_u16(stk, rand, depth - 1)).await as u16
        } else {
            stk.run(|stk| count_u128(stk, rand, depth - 1)).await as u16
        };
        v.wrapping_add(c) as usize
    }

    async fn count_u128(stk: &mut Stk, rand: &mut Rand, depth: usize) -> usize {
        let v = rand.next() as u128;
        if depth == 0 {
            return v as usize;
        }
        let c = if (rand.next() & 1) == 0 {
            stk.run(|stk| count_u16(stk, rand, depth - 1)).await as u128
        } else {
            stk.run(|stk| count_u128(stk, rand, depth - 1)).await as u128
        };
        v.wrapping_add(c) as usize
    }

    let mut rand = Rand::new();
    let mut stack = Stack::new();
    #[cfg(miri)]
    let depth = 10;
    #[cfg(not(miri))]
    let depth = 1000;
    stack
        .enter(|stk| count_u128(stk, &mut rand, depth))
        .finish();
}

// miri doesn't support epoll properly
#[cfg(not(miri))]
#[tokio::test]
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

    assert_eq!(str, include_str!("../Cargo.toml"));
}

// miri doesn't support epoll properly
#[cfg(not(miri))]
#[tokio::test]
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

        assert_eq!(str, include_str!("../Cargo.toml"));
    })
    .await
    .unwrap();
}
