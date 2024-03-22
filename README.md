[![codecov](https://codecov.io/gh/DelSkayn/reblessive/graph/badge.svg?token=A2DZXD34AZ)](https://codecov.io/gh/DelSkayn/reblessive)


# Reblessive

A heap allocated runtime for deeply recursive algorithms.

Turn your cursed recursive algorithm into a blessed heap allocated structure which won't 
overflow the stack, regardless of depth.

## What is this crate for?

There are some types of algorithms which are easiest to write as a recursive algorithm. 
Examples include a recursive decent parsers and tree-walking interpreters. 
These algorithms often need to keep track of complex stack of state and are therefore easiest to write as a set of recursive function calling each other.
This does however have a major downside: The stack can be rather limited.
Especially when the input of a algorithm is externally controlled, implementing it as a recursive algorithm is asking for stack overflows. 

This library is an attempt to solve that issue.
It provides a small executor which is able to efficiently allocate new futures in a stack like order and then drive them in a single loop.
With these executors you can write your recursive algorithm as a set of futures. 
The executor will then, whenever a function needs to call another, pause the current future to execute the newly scheduled future.
This allows one to implement your algorithm in a way that still looks recursive but won't run out of stack if recursing very deep.


## Example

```rust
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
    let res = stack.enter(|ctx| heavy_fibbo(ctx, 20)).finish();
    println!("result: {res}");

    assert_eq!(res, 10946);

    // Reblessive can also make any recursive function interuptable.
    let mut runner = stack.enter(|ctx| heavy_fibbo(ctx, 60));

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
```
