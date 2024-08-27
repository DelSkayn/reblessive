fn main() {
    println!("cargo::rerun-if-env-changed=CARGO_FEATURE_BYPASS");

    if std::env::vars().any(|x| dbg!(x.0) == "CARGO_FEATURE_BYPASS") {
        println!("cargo::warning=Compiled reblessive with `bypass` feature enabled, this disables reblessive's stack overflow prevention!")
    }
}
