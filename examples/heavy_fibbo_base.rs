use std::mem::MaybeUninit;

fn heavy_fibbo(n: usize) -> usize {
    let mut ballast: MaybeUninit<[u8; 1024 * 1024]> = std::mem::MaybeUninit::uninit();
    std::hint::black_box(&mut ballast);

    match n {
        0 => 1,
        1 => 1,
        x => heavy_fibbo(x - 1) + heavy_fibbo(x - 2),
    }
}

fn main() {
    let res = heavy_fibbo(20);

    assert_eq!(res, 10946)
}
