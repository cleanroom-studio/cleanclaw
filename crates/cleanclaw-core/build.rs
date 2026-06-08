fn main() {
    println!(
        "cargo:rustc-env=CLEANCLAW_BUILD_VERSION={}",
        option_env!("CLEANCLAW_BUILD_VERSION").unwrap_or("dev")
    );
    println!(
        "cargo:rustc-env=CLEANCLAW_BUILD_COMMIT={}",
        option_env!("CLEANCLAW_BUILD_COMMIT").unwrap_or("unknown")
    );
    println!(
        "cargo:rustc-env=CLEANCLAW_BUILD_DATE={}",
        option_env!("CLEANCLAW_BUILD_DATE").unwrap_or("unknown")
    );
}
