use std::{
    mem::MaybeUninit,
    time::{Duration, Instant},
};

use reblessive::{Stack, Stk};

async fn heavy_fibbo(ctx: &mut Stk, n: usize) -> usize {
    // An extra stack allocation to simulate a more complex function.
    let mut ballast: MaybeUninit<[u8; 1024 * 1024]> = std::mem::MaybeUninit::uninit();
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

fn main() {
    // Create a stack to run the function in.
    let mut stack = Stack::new();

    // run the function to completion on the stack.
    let res = stack.run(|ctx| heavy_fibbo(ctx, 20)).finish();
    println!("result: {res}");

    assert_eq!(res, 10946);

    // Reblessive can also make any recursive function interuptable.
    let mut runner = stack.run(|ctx| heavy_fibbo(ctx, 60));

    let start = Instant::now();
    loop {
        // run the function forward by a step.
        // If this returned Some than the function completed.
        if let Some(x) = runner.step() {
            println!("finished: {x}")
        }
        // We didn't complete the computation in time so we can just drop the runner and stop the
        // function.
        if start.elapsed() > Duration::from_secs(3) {
            println!("Timed out!");
            break;
        }
    }
}
