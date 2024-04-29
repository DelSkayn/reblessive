use criterion::{criterion_group, criterion_main, Criterion};
use reblessive::{Stack, Stk};

async fn heavy_fibbo(ctx: &mut Stk, n: usize) -> usize {
    match n {
        0 => 1,
        1 => 1,
        x => {
            ctx.run(move |ctx| heavy_fibbo(ctx, x - 1)).await
                + ctx.run(move |ctx| heavy_fibbo(ctx, x - 2)).await
        }
    }
}

fn bench_fibbo(c: &mut Criterion) {
    c.bench_function("fib 15", |b| {
        b.iter(|| {
            // Create a stack to run the function in.
            let mut stack = Stack::new();

            // run the function to completion on the stack.
            let res = stack.enter(|ctx| heavy_fibbo(ctx, 15)).finish();
            assert_eq!(res, 987);
        })
    });

    c.bench_function("fib 20", |b| {
        b.iter(|| {
            // Create a stack to run the function in.
            let mut stack = Stack::new();

            // run the function to completion on the stack.
            let res = stack.enter(|ctx| heavy_fibbo(ctx, 20)).finish();
            assert_eq!(res, 10946);
        })
    });

    c.bench_function("fib 25", |b| {
        b.iter(|| {
            // Create a stack to run the function in.
            let mut stack = Stack::new();

            // run the function to completion on the stack.
            let res = stack.enter(|ctx| heavy_fibbo(ctx, 30)).finish();
            assert_eq!(res, 1346269);
        })
    });
}

fn bench_fibbo_async(c: &mut Criterion) {
    c.bench_function("fib async 15", |b| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        b.iter(|| {
            rt.block_on(async {
                // Create a stack to run the function in.
                let mut stack = Stack::new();

                // run the function to completion on the stack.
                let res = stack.enter(|ctx| heavy_fibbo(ctx, 15)).finish_async().await;
                assert_eq!(res, 987);
            })
        })
    });

    c.bench_function("fib async 20", |b| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        b.iter(|| {
            rt.block_on(async {
                // Create a stack to run the function in.
                let mut stack = Stack::new();

                // run the function to completion on the stack.
                let res = stack.enter(|ctx| heavy_fibbo(ctx, 20)).finish_async().await;
                assert_eq!(res, 10946);
            })
        })
    });

    c.bench_function("fib async 25", |b| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        b.iter(|| {
            rt.block_on(async {
                // Create a stack to run the function in.
                let mut stack = Stack::new();

                // run the function to completion on the stack.
                let res = stack.enter(|ctx| heavy_fibbo(ctx, 30)).finish_async().await;
                assert_eq!(res, 1346269);
            })
        })
    });
}

criterion_group!(benches, bench_fibbo, bench_fibbo_async);
criterion_main!(benches);
