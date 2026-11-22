fn main() {
    #[cfg(windows)]
    {
        use embed_manifest::{embed_manifest, new_manifest, manifest::ExecutionLevel};
        let is_release = std::env::var("PROFILE").unwrap_or_default() == "release";
        let is_test = std::env::var("ESTUS_TEST").is_ok();
        let level = if is_release && !is_test {
            ExecutionLevel::RequireAdministrator
        } else {
            ExecutionLevel::AsInvoker
        };
        embed_manifest(
            new_manifest("Estus")
                .requested_execution_level(level)
        ).expect("unable to embed manifest file");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ESTUS_TEST");
}
