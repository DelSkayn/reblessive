use std::mem::MaybeUninit;

use reblessive::{Stack, Stk};

async fn deep_read(ctx: &mut Stk, n: usize) -> String {
    let mut ballast: MaybeUninit<[u8; 1024 * 1024]> = std::mem::MaybeUninit::uninit();
    std::hint::black_box(&mut ballast);

    if n == 0 {
        tokio::fs::read_to_string("./Cargo.toml").await.unwrap()
    } else {
        ctx.run(|ctx| deep_read(ctx, n - 1)).await
    }
}

#[tokio::main]
async fn main() {
    let mut stack = Stack::new();

    let str = stack.run(|ctx| deep_read(ctx, 20)).finish_async().await;

    println!("{}", str);
}
